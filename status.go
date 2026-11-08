package main

// Service status: list, the status table and JSON, and the checks behind
// them.

import (
	"encoding/json"
	"fmt"
	"net"
	"strings"
	"time"
)

// StatusRow is one service's state, as `status --json` and the web API
// show it.
type StatusRow struct {
	ID          string
	Name        string
	Plane       string
	Category    string
	Kind        string
	State       string
	Detail      string
	URL         *string
	Access      *string
	Description *string
	Ports       []string
	Repo        *string
	Controls    []string
	Optional    bool
}

func (r StatusRow) json() jsonObject {
	return jsonObject{
		{"id", r.ID}, {"name", r.Name}, {"plane", r.Plane}, {"category", r.Category},
		{"kind", r.Kind}, {"state", r.State}, {"detail", r.Detail}, {"url", r.URL},
		{"access", r.Access}, {"description", r.Description}, {"ports", r.Ports},
		{"repo", r.Repo}, {"controls", r.Controls}, {"optional", r.Optional},
	}
}

func statusRowsJSON(rows []StatusRow) string {
	items := make([]any, len(rows))
	for i, row := range rows {
		items[i] = row.json()
	}
	return prettyJSON(items)
}

func cmdStatus(ctx *Context, args []string) (int, error) {
	asJSON := false
	for _, arg := range args {
		if arg == "--json" {
			asJSON = true
		}
	}
	rows := statusRows(ctx)
	if asJSON {
		fmt.Println(statusRowsJSON(rows))
	} else {
		printStatusTable(rows)
	}
	return 0, nil
}

func cmdList(ctx *Context) (int, error) {
	for _, s := range ctx.Manifest.Service {
		access := "-"
		if s.URL != nil {
			access = *s.URL
		} else if s.Access != nil {
			access = *s.Access
		}
		fmt.Printf("%-18s %-13s %s\n", s.ID, s.Kind, access)
	}
	return 0, nil
}

func printStatusTable(rows []StatusRow) {
	fmt.Printf("%-9s %-8s %-13s %-22s DETAIL\n", "STATE", "PLANE", "KIND", "SERVICE")
	for _, row := range rows {
		fmt.Printf("%-9s %-8s %-13s %-22s %s\n", row.State, row.Plane, row.Kind, row.Name, row.Detail)
	}
}

func statusRows(ctx *Context) []StatusRow {
	rows := make([]StatusRow, 0, len(ctx.Manifest.Service))
	for _, s := range ctx.Manifest.Service {
		state, detail := serviceStatus(s)
		ports := s.Ports
		if ports == nil {
			ports = []string{}
		}
		controls := s.Controls
		if controls == nil {
			controls = []string{}
		}
		rows = append(rows, StatusRow{
			ID: s.ID, Name: s.Name, Plane: deref(s.Plane, "private"), Category: deref(s.Category, "-"),
			Kind: s.Kind, State: state, Detail: detail, URL: s.URL, Access: s.Access,
			Description: s.Description, Ports: ports, Repo: s.Repo, Controls: controls,
			Optional: s.Optional != nil && *s.Optional,
		})
	}
	return rows
}

func serviceStatus(s Service) (string, string) {
	switch s.Kind {
	case "compose":
		return composeStatus(s)
	case "systemd":
		return systemdStatus(deref(s.Unit, ""))
	case "user-systemd":
		return userSystemdStatus(deref(s.User, ""), deref(s.Unit, ""))
	case "port", "manual":
		port := uint16(0)
		if s.Port != nil {
			port = *s.Port
		}
		return portStatus(deref(s.Host, "127.0.0.1"), port)
	}
	return "unknown", "unknown kind " + s.Kind
}

func composeStatus(s Service) (string, string) {
	if len(s.Containers) == 0 {
		return "unknown", "no containers listed"
	}
	online := 0
	var details []string
	for _, container := range s.Containers {
		out := runCommand("docker", []string{"inspect", "--format", "{{json .State}}", container}, "", nil)
		if out.Code != 0 {
			details = append(details, container+": missing")
			continue
		}
		var state map[string]any
		if json.Unmarshal([]byte(out.Text), &state) != nil {
			state = nil
		}
		if running, _ := state["Running"].(bool); running {
			online++
			health, _ := state["Health"].(map[string]any)
			if status, ok := health["Status"].(string); ok {
				details = append(details, container+": "+status)
			} else {
				details = append(details, container+": running")
			}
			continue
		}
		status, ok := state["Status"].(string)
		if !ok {
			status = "stopped"
		}
		details = append(details, container+": "+status)
	}
	state := "offline"
	if online == len(s.Containers) {
		state = "online"
	} else if online > 0 {
		state = "partial"
	}
	return state, strings.Join(details, ", ")
}

func firstLine(text string) string {
	lines := rustLines(text)
	if len(lines) == 0 {
		return "unknown"
	}
	return lines[0]
}

func systemdStatus(unit string) (string, string) {
	if unit == "" {
		return "unknown", "missing unit"
	}
	active := runCommand("systemctl", []string{"is-active", unit}, "", nil)
	enabled := runCommand("systemctl", []string{"is-enabled", unit}, "", nil)
	activeText, enabledText := firstLine(active.Text), firstLine(enabled.Text)
	state := "unknown"
	if active.Code == 0 {
		state = "online"
	} else if activeText == "inactive" || activeText == "failed" {
		state = "offline"
	}
	return state, activeText + ", " + enabledText
}

func userSystemdStatus(user, unit string) (string, string) {
	if unit == "" {
		return "unknown", "missing unit"
	}
	prefix, ok := userSystemdPrefix(user)
	if !ok {
		return "unknown", fmt.Sprintf("user %s not found", user)
	}
	active := runPrefixedCommand(prefix, "systemctl", []string{"--user", "is-active", unit})
	enabled := runPrefixedCommand(prefix, "systemctl", []string{"--user", "is-enabled", unit})
	activeText, enabledText := firstLine(active.Text), firstLine(enabled.Text)
	state := "unknown"
	switch {
	case active.Code == 0:
		state = "online"
	case activeText == "inactive" || activeText == "failed":
		state = "offline"
	case strings.Contains(activeText, "not-found") || strings.Contains(activeText, "not found"):
		state = "missing"
	}
	return state, activeText + ", " + enabledText
}

func portStatus(host string, port uint16) (string, string) {
	if port == 0 {
		return "unknown", "missing port"
	}
	target := fmt.Sprintf("%s:%d", host, port)
	var ips []net.IP
	if ip := net.ParseIP(host); ip != nil {
		ips = []net.IP{ip}
	} else {
		found, err := net.LookupIP(host)
		if err != nil {
			return "unknown", target + " did not resolve"
		}
		ips = found
	}
	var v4 net.IP
	for _, ip := range ips {
		if ip.To4() != nil && !strings.Contains(host, ":") {
			v4 = ip.To4()
			break
		}
	}
	if v4 == nil {
		return "unknown", target + " has no IPv4 address"
	}
	conn, err := net.DialTimeout("tcp4", net.JoinHostPort(v4.String(), fmt.Sprint(port)), 700*time.Millisecond)
	if err != nil {
		return "offline", fmt.Sprintf("%s closed (%s)", target, errorKind(err))
	}
	conn.Close()
	return "online", target + " accepts connections"
}
