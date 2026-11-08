package main

// doctor: a read-only host health scan; release-check.

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"strconv"
	"strings"
)

type doctorFinding struct {
	severity string // ok, info, warn, error
	check    string
	message  string
	detail   *string
}

func (f doctorFinding) json() jsonObject {
	o := jsonObject{{"severity", f.severity}, {"check", f.check}, {"message", f.message}}
	if f.detail != nil {
		o = append(o, jsonField{"detail", *f.detail})
	}
	return o
}

func findingOK(check, message string) doctorFinding {
	return doctorFinding{severity: "ok", check: check, message: message}
}

func finding(severity, check, message, detail string) doctorFinding {
	return doctorFinding{severity: severity, check: check, message: message, detail: &detail}
}

const doctorHelp = `Usage:
  plugroot doctor [--json] [--strict]

Runs a read-only host health scan for Plugroot boundaries, declared services,
firewall exposure, Tailscale Funnel, wildcard listeners, stale Docker/tmux/FUSE
state, failed systemd units, and basic resource pressure.

By default doctor exits nonzero only for errors. --strict also fails on warnings.
`

func cmdDoctor(ctx *Context, args []string) (int, error) {
	asJSON, strict := false, false
	for _, arg := range args {
		switch arg {
		case "-h", "--help":
			fmt.Print(doctorHelp + "\n")
			return 0, nil
		case "--json":
			asJSON = true
		case "--strict":
			strict = true
		default:
			fmt.Fprintln(os.Stderr, "usage: plugroot doctor [--json] [--strict]")
			return 2, nil
		}
	}
	findings, err := doctorFindings(ctx)
	if err != nil {
		return 0, err
	}
	if asJSON {
		items := make([]any, len(findings))
		for i, f := range findings {
			items[i] = f.json()
		}
		fmt.Println(prettyJSON(items))
	} else {
		printDoctorReport(findings)
	}
	errorsFound, warnings := countSeverity(findings, "error"), countSeverity(findings, "warn")
	if errorsFound > 0 || strict && warnings > 0 {
		return 1, nil
	}
	return 0, nil
}

func countSeverity(findings []doctorFinding, severity string) int {
	n := 0
	for _, f := range findings {
		if f.severity == severity {
			n++
		}
	}
	return n
}

func cmdReleaseCheck(ctx *Context) (int, error) {
	failed := false
	boundary, err := boundaryFindings(ctx)
	if err != nil {
		return 0, err
	}
	if len(boundary) == 0 {
		fmt.Println("boundary: ok")
	} else {
		failed = true
		fmt.Fprintf(os.Stderr, "boundary: found %d issue(s)\n", len(boundary))
		for _, f := range boundary {
			location := "-"
			if f.path != nil {
				location = *f.path
			}
			fmt.Fprintf(os.Stderr, "%s: %s: %s\n", f.severity, location, f.message)
		}
	}
	audit, err := auditPublic(ctx.Root)
	switch {
	case err != nil:
		failed = true
		fmt.Fprintf(os.Stderr, "audit-public: failed: %s\n", err)
	case len(audit) == 0:
		fmt.Println("audit-public: ok")
	default:
		failed = true
		fmt.Fprintf(os.Stderr, "audit-public: found %d issue(s)\n", len(audit))
		for _, f := range audit {
			fmt.Fprintln(os.Stderr, f.String())
		}
	}
	switch detail, err := gitDirtyDetails(ctx.Root); {
	case err != nil:
		failed = true
		fmt.Fprintf(os.Stderr, "git: could not check code checkout: %s\n", err)
	case detail == "":
		fmt.Println("git: code checkout clean")
	default:
		failed = true
		fmt.Fprintf(os.Stderr, "git: code checkout has uncommitted changes\n%s\n", detail)
	}
	for _, r := range ctx.Manifest.Repo {
		switch detail, err := gitDirtyDetails(r.Path); {
		case err != nil:
			failed = true
			fmt.Fprintf(os.Stderr, "git: could not check %s: %s\n", r.ID, err)
		case detail == "":
			fmt.Printf("git: %s clean\n", r.ID)
		default:
			failed = true
			fmt.Fprintf(os.Stderr, "git: %s has uncommitted changes\n%s\n", r.ID, detail)
		}
	}
	if failed {
		return 1, nil
	}
	fmt.Println("release-check: ok")
	return 0, nil
}

func printDoctorReport(findings []doctorFinding) {
	fmt.Printf("doctor: %d check(s), %d error(s), %d warning(s)\n", len(findings),
		countSeverity(findings, "error"), countSeverity(findings, "warn"))
	fmt.Printf("%-7s %-18s MESSAGE\n", "STATE", "CHECK")
	for _, f := range findings {
		fmt.Printf("%-7s %-18s %s\n", f.severity, f.check, f.message)
		if f.detail != nil {
			for _, line := range rustLines(*f.detail) {
				fmt.Printf("%-7s %-18s %s\n", "", "", line)
			}
		}
	}
}

func doctorFindings(ctx *Context) ([]doctorFinding, error) {
	var findings []doctorFinding
	boundary, err := doctorBoundary(ctx)
	if err != nil {
		return nil, err
	}
	findings = append(findings, boundary...)
	findings = append(findings, doctorAudit(ctx)...)
	findings = append(findings, doctorGit(ctx)...)
	findings = append(findings, doctorDeclaredServices(ctx))
	findings = append(findings, doctorLoad(), doctorMemory(), doctorRootDisk())
	findings = append(findings, doctorFailedUnits()...)
	findings = append(findings, doctorUFW(), doctorTailscale(), doctorListeners(ctx))
	findings = append(findings, doctorDocker(ctx)...)
	findings = append(findings, doctorTmux(), doctorFuseMounts())
	return findings, nil
}

func doctorBoundary(ctx *Context) ([]doctorFinding, error) {
	boundary, err := boundaryFindings(ctx)
	if err != nil {
		return nil, err
	}
	if len(boundary) == 0 {
		return []doctorFinding{findingOK("boundary", "code and private state roots are separated")}, nil
	}
	var out []doctorFinding
	for _, f := range boundary {
		detail := "-"
		if f.path != nil {
			detail = *f.path
		}
		severity := "warn"
		if f.severity == "error" {
			severity = "error"
		}
		out = append(out, finding(severity, "boundary", f.message, detail))
	}
	return out, nil
}

func doctorAudit(ctx *Context) []doctorFinding {
	audit, err := auditPublic(ctx.Root)
	if err != nil {
		return []doctorFinding{finding("warn", "audit-public", "could not audit tracked files", err.Error())}
	}
	if len(audit) == 0 {
		return []doctorFinding{findingOK("audit-public", "tracked files passed private-state audit")}
	}
	var lines []string
	for i, f := range audit {
		if i == 12 {
			break
		}
		lines = append(lines, f.String())
	}
	return []doctorFinding{finding("error", "audit-public",
		fmt.Sprintf("tracked files have %d private-state issue(s)", len(audit)), strings.Join(lines, "\n"))}
}

func doctorGit(ctx *Context) []doctorFinding {
	var out []doctorFinding
	switch detail, err := gitDirtyDetails(ctx.Root); {
	case err != nil:
		out = append(out, finding("warn", "git", "could not check code checkout cleanliness", err.Error()))
	case detail == "":
		out = append(out, findingOK("git", "code checkout is clean"))
	default:
		out = append(out, finding("warn", "git", "code checkout has uncommitted changes", detail))
	}
	var dirty, failed []string
	for _, r := range ctx.Manifest.Repo {
		switch detail, err := gitDirtyDetails(r.Path); {
		case err != nil:
			failed = append(failed, fmt.Sprintf("%s: %s", r.ID, err))
		case detail != "":
			dirty = append(dirty, fmt.Sprintf("%s:\n%s", r.ID, detail))
		}
	}
	if len(dirty) == 0 && len(failed) == 0 {
		return append(out, findingOK("repo-git", "managed repo checkouts are clean"))
	}
	if len(dirty) > 0 {
		out = append(out, finding("warn", "repo-git", fmt.Sprintf("%d managed repo checkout(s) are dirty", len(dirty)), limitLines(dirty, 12)))
	}
	if len(failed) > 0 {
		out = append(out, finding("warn", "repo-git", fmt.Sprintf("could not inspect %d managed repo checkout(s)", len(failed)), limitLines(failed, 12)))
	}
	return out
}

func doctorDeclaredServices(ctx *Context) doctorFinding {
	var attention []string
	for _, row := range statusRows(ctx) {
		if row.State == "online" {
			continue
		}
		if row.Optional && (row.State == "offline" || row.State == "missing") {
			continue
		}
		attention = append(attention, fmt.Sprintf("%s: %s (%s)", row.ID, row.State, row.Detail))
	}
	if len(attention) == 0 {
		return findingOK("services", "declared non-optional services are online")
	}
	return finding("warn", "services", fmt.Sprintf("%d declared service(s) need attention", len(attention)), limitLines(attention, 16))
}

func doctorLoad() doctorFinding {
	raw, err := readText("/proc/loadavg")
	if err != nil {
		return finding("info", "load", "could not read system load", "/proc/loadavg unavailable")
	}
	fields := strings.Fields(raw)
	var load1 float64
	ok := len(fields) > 0
	if ok {
		load1, ok = parseRustFloat(fields[0])
	}
	if !ok {
		return finding("info", "load", "could not parse system load", raw)
	}
	cpus := float64(availableParallelism())
	if load1 > cpus*1.5 {
		return finding("warn", "load", "one-minute load is high", fmt.Sprintf("load1=%.2f, cpu_count=%.0f", load1, cpus))
	}
	return findingOK("load", fmt.Sprintf("one-minute load is %.2f across %.0f CPU(s)", load1, cpus))
}

// availableParallelism is what Rust's available_parallelism reported: the
// CPUs this process may run on (its affinity mask).
func availableParallelism() int {
	if n := runtime.NumCPU(); n > 0 {
		return n
	}
	return 1
}

// parseRustFloat accepts what Rust's f64 parsing accepts for /proc/loadavg
// numbers: decimal digits with an optional fraction.
func parseRustFloat(s string) (float64, bool) {
	if s == "" || strings.ContainsAny(s, "_xXpP") {
		return 0, false
	}
	f, err := strconv.ParseFloat(s, 64)
	return f, err == nil
}

func doctorMemory() doctorFinding {
	raw, err := readText("/proc/meminfo")
	if err != nil {
		return finding("info", "memory", "could not read memory pressure", "/proc/meminfo unavailable")
	}
	total, ok := meminfoKB(raw, "MemTotal")
	if !ok {
		return finding("info", "memory", "could not parse memory total", "/proc/meminfo missing MemTotal")
	}
	available, ok := meminfoKB(raw, "MemAvailable")
	if !ok {
		return finding("info", "memory", "could not parse available memory", "/proc/meminfo missing MemAvailable")
	}
	percent := float64(available) * 100.0 / float64(total)
	detail := fmt.Sprintf("%.1f%% available (%d MiB of %d MiB)", percent, available/1024, total/1024)
	if percent < 10.0 {
		return finding("warn", "memory", "available memory is low", detail)
	}
	return findingOK("memory", detail)
}

func meminfoKB(text, key string) (uint64, bool) {
	for _, line := range rustLines(text) {
		name, rest, ok := strings.Cut(line, ":")
		if !ok || name != key {
			continue
		}
		fields := strings.Fields(rest)
		if len(fields) == 0 {
			continue
		}
		if n, ok := parseUnsigned(fields[0], 64); ok {
			return n, true
		}
	}
	return 0, false
}

func doctorRootDisk() doctorFinding {
	out := runCommand("df", []string{"-P", "/"}, "", nil)
	if out.Code != 0 {
		return finding("warn", "disk", "could not inspect root filesystem usage", out.Text)
	}
	lines := rustLines(out.Text)
	if len(lines) < 2 {
		return finding("info", "disk", "could not parse root filesystem usage", out.Text)
	}
	line := lines[1]
	columns := strings.Fields(line)
	var use uint64
	ok := len(columns) > 4
	if ok {
		use, ok = parseUnsigned(strings.TrimRight(columns[4], "%"), 8)
	}
	if !ok {
		return finding("info", "disk", "could not parse root filesystem percentage", line)
	}
	if use >= 90 {
		return finding("warn", "disk", "root filesystem is close to full", line)
	}
	return findingOK("disk", fmt.Sprintf("root filesystem is %d%% used", use))
}

func doctorFailedUnits() []doctorFinding {
	var out []doctorFinding
	system := runCommand("systemctl", []string{"--failed", "--no-legend", "--no-pager"}, "", nil)
	if system.Code == 0 {
		if text := strings.TrimSpace(system.Text); text == "" {
			out = append(out, findingOK("systemd", "no failed system units"))
		} else {
			out = append(out, finding("warn", "systemd", "failed system units are present", text))
		}
	} else {
		out = append(out, finding("info", "systemd", "could not inspect failed system units", system.Text))
	}
	user := runCommand("systemctl", []string{"--user", "--failed", "--no-legend", "--no-pager"}, "", nil)
	if user.Code == 0 {
		if text := strings.TrimSpace(user.Text); text == "" {
			out = append(out, findingOK("user-systemd", "no failed user units"))
		} else {
			out = append(out, finding("warn", "user-systemd", "failed user units are present", text))
		}
	} else {
		out = append(out, finding("info", "user-systemd", "could not inspect failed user units", user.Text))
	}
	return out
}

func doctorUFW() doctorFinding {
	out := runReadCommand("ufw", []string{"status", "verbose"})
	if out.Code != 0 {
		return finding("warn", "ufw", "could not inspect firewall status", out.Text)
	}
	lower := asciiLower(out.Text)
	if strings.Contains(lower, "status: inactive") {
		return finding("error", "ufw", "firewall is inactive", out.Text)
	}
	if !strings.Contains(lower, "status: active") {
		return finding("info", "ufw", "firewall status was not recognized", out.Text)
	}
	broad := broadUFWAllowRules(out.Text)
	if len(broad) == 0 {
		return findingOK("ufw", "firewall is active with no broad inbound allow rules detected")
	}
	return finding("warn", "ufw", fmt.Sprintf("%d broad inbound allow rule(s) detected", len(broad)), limitLines(broad, 12))
}

func doctorTailscale() doctorFinding {
	serve := runCommand("tailscale", []string{"serve", "status"}, "", nil)
	funnel := runCommand("tailscale", []string{"funnel", "status"}, "", nil)
	if serve.Code == 127 && funnel.Code == 127 {
		return finding("info", "tailscale", "tailscale command is unavailable", serve.Text)
	}
	var details []string
	if serve.Code == 0 && serve.Text != "" {
		details = append(details, serve.Text)
	}
	if funnel.Code == 0 && funnel.Text != "" && funnel.Text != serve.Text {
		details = append(details, funnel.Text)
	}
	combined := strings.Join(details, "\n")
	switch {
	case tailscaleTextHasPublicFunnel(combined):
		return finding("error", "tailscale", "Tailscale Funnel appears to be enabled", combined)
	case serve.Code == 0 || funnel.Code == 0:
		return findingOK("tailscale", "no public Tailscale Funnel exposure detected")
	}
	return finding("info", "tailscale", "could not inspect Tailscale Serve/Funnel", serve.Text+"\n"+funnel.Text)
}

func doctorListeners(ctx *Context) doctorFinding {
	expected := expectedManifestTCPPorts(ctx)
	out := runCommand("ss", []string{"-ltnupH"}, "", nil)
	if out.Code != 0 {
		return finding("warn", "listeners", "could not inspect listening sockets", out.Text)
	}
	unexpected := unexpectedWildcardTCPListeners(out.Text, expected)
	if len(unexpected) == 0 {
		return findingOK("listeners", "no unexpected wildcard TCP listeners detected")
	}
	return finding("warn", "listeners", fmt.Sprintf("%d unexpected wildcard TCP listener(s) detected", len(unexpected)), limitLines(unexpected, 16))
}

func doctorTmux() doctorFinding {
	out := runCommand("tmux", []string{"list-sessions", "-F", "#{session_name}\t#{session_attached}\t#{t:session_activity}"}, "", nil)
	if out.Code != 0 {
		if strings.Contains(asciiLower(out.Text), "no server running") {
			return findingOK("tmux", "no tmux server is running")
		}
		return finding("info", "tmux", "could not inspect tmux sessions", out.Text)
	}
	retired := retiredTmuxSessions(out.Text)
	if len(retired) == 0 {
		return findingOK("tmux", "no retired codex/openclaw tmux sessions detected")
	}
	return finding("warn", "tmux", fmt.Sprintf("%d retired tmux session(s) still exist", len(retired)), limitLines(retired, 12))
}

func doctorFuseMounts() doctorFinding {
	out := runCommand("findmnt", []string{"-rn", "-o", "TARGET,FSTYPE,SOURCE"}, "", nil)
	if out.Code != 0 {
		return finding("info", "fuse", "could not inspect mounted filesystems", out.Text)
	}
	var stale []string
	for _, line := range rustLines(out.Text) {
		columns := strings.Fields(line)
		if len(columns) == 0 {
			continue
		}
		target, fstype := columns[0], ""
		if len(columns) > 1 {
			fstype = columns[1]
		}
		if !strings.HasPrefix(fstype, "fuse") {
			continue
		}
		stat := runCommand("timeout", []string{"2", "stat", "-c", "%F", target}, "", nil)
		if stat.Code != 0 {
			stale = append(stale, target+": "+stat.Text)
		}
	}
	if len(stale) == 0 {
		return findingOK("fuse", "no stale FUSE mounts detected")
	}
	return finding("warn", "fuse", fmt.Sprintf("%d stale FUSE mount(s) detected", len(stale)), limitLines(stale, 8))
}

func broadUFWAllowRules(text string) []string {
	var out []string
	for _, raw := range rustLines(text) {
		line := strings.TrimSpace(raw)
		if strings.Contains(line, "ALLOW IN") && strings.Contains(line, "Anywhere") &&
			!strings.Contains(line, " on tailscale0") && !strings.Contains(line, "41641/udp") {
			out = append(out, line)
		}
	}
	return out
}

func tailscaleTextHasPublicFunnel(text string) bool {
	for _, line := range rustLines(text) {
		lower := asciiLower(line)
		if strings.Contains(lower, "funnel") && !strings.Contains(lower, "funnel off") &&
			!strings.Contains(lower, "funnel is off") && !strings.Contains(lower, "disabled") {
			return true
		}
	}
	return false
}

func expectedManifestTCPPorts(ctx *Context) map[uint16]bool {
	ports := map[uint16]bool{}
	for _, s := range ctx.Manifest.Service {
		if (s.Kind == "port" || s.Kind == "manual") && s.Port != nil {
			ports[*s.Port] = true
		}
		for _, descriptor := range s.Ports {
			addTCPPortsFromDescriptor(descriptor, ports)
		}
	}
	return ports
}

func addTCPPortsFromDescriptor(descriptor string, ports map[uint16]bool) {
	tokens := strings.FieldsFunc(descriptor, func(r rune) bool { return isRustWhitespace(r) || r == ',' })
	// Rust's split kept empty tokens, which never hold a "/", so skipping
	// them changes nothing.
	for _, token := range tokens {
		portRange, protocol, ok := strings.Cut(token, "/")
		if !ok || !strings.HasPrefix(asciiLower(protocol), "tcp") {
			continue
		}
		addPortRange(portRange, ports)
	}
}

func isRustWhitespace(r rune) bool {
	switch r {
	case ' ', '\t', '\n', '\v', '\f', '\r', 0x85, 0xA0, 0x1680, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000:
		return true
	}
	return r >= 0x2000 && r <= 0x200A
}

func addPortRange(portRange string, ports map[uint16]bool) {
	if start, end, ok := strings.Cut(portRange, "-"); ok {
		s, ok1 := parseUnsigned(start, 16)
		e, ok2 := parseUnsigned(end, 16)
		if !ok1 || !ok2 || s > e {
			return
		}
		for p := s; p <= e; p++ {
			ports[uint16(p)] = true
		}
		return
	}
	if p, ok := parseUnsigned(portRange, 16); ok {
		ports[uint16(p)] = true
	}
}

func unexpectedWildcardTCPListeners(text string, expected map[uint16]bool) []string {
	var out []string
	for _, line := range rustLines(text) {
		columns := strings.Fields(line)
		if len(columns) == 0 || columns[0] != "tcp" || len(columns) < 5 {
			continue
		}
		local := columns[4]
		if !isWildcardListener(local) {
			continue
		}
		port, ok := listenerPort(local)
		if !ok || expected[port] {
			continue
		}
		out = append(out, strings.TrimSpace(line))
	}
	return out
}

func isWildcardListener(local string) bool {
	return strings.HasPrefix(local, "0.0.0.0:") || strings.HasPrefix(local, "[::]:") || strings.HasPrefix(local, "*:")
}

func listenerPort(local string) (uint16, bool) {
	i := strings.LastIndexByte(local, ':')
	if i < 0 {
		return 0, false
	}
	p, ok := parseUnsigned(local[i+1:], 16)
	return uint16(p), ok
}

func expectedContainerNames(ctx *Context) map[string]bool {
	names := map[string]bool{}
	for _, s := range ctx.Manifest.Service {
		for _, c := range s.Containers {
			names[c] = true
		}
	}
	return names
}

func retiredTmuxSessions(text string) []string {
	var out []string
	for _, line := range rustLines(text) {
		columns := strings.Split(line, "\t")
		get := func(i int) string {
			if i < len(columns) {
				return columns[i]
			}
			return ""
		}
		name, attached, activity := get(0), get(1), get(2)
		if attached == "0" && (strings.HasPrefix(name, "codex-") || strings.HasPrefix(name, "openclaw")) {
			out = append(out, fmt.Sprintf("%s: detached, last active %s", name, activity))
		}
	}
	return out
}

// runReadCommand runs a read-only command as root: directly when root,
// else through `sudo -n`, else as this user.
func runReadCommand(program string, args []string) CmdOutput {
	if isRoot() {
		return runCommand(program, args, "", nil)
	}
	sudo := runCommand("sudo", append([]string{"-n", program}, args...), "", nil)
	if sudo.Code == 0 {
		return sudo
	}
	return runCommand(program, args, "", nil)
}

func doctorDocker(ctx *Context) []doctorFinding {
	var out []doctorFinding
	out = append(out, doctorDockerContainers(ctx)...)
	out = append(out, doctorDockerNetworks(), doctorDockerVolumes(), doctorDockerReclaimable())
	return out
}

func nonBlankLines(text string) []string {
	var out []string
	for _, line := range rustLines(text) {
		if strings.TrimSpace(line) != "" {
			out = append(out, line)
		}
	}
	return out
}

func doctorDockerContainers(ctx *Context) []doctorFinding {
	out := runCommand("docker", []string{"ps", "-a", "--format", "{{.Names}}\t{{.Status}}\t{{.Label \"com.docker.compose.project\"}}"}, "", nil)
	if out.Code != 0 {
		return []doctorFinding{finding("info", "docker", "could not inspect Docker containers", out.Text)}
	}
	expected := expectedContainerNames(ctx)
	var stopped, unmanaged []string
	for _, line := range nonBlankLines(out.Text) {
		parts := strings.SplitN(line, "\t", 3)
		get := func(i int) string {
			if i < len(parts) {
				return parts[i]
			}
			return ""
		}
		name, status, project := get(0), get(1), get(2)
		if !strings.HasPrefix(status, "Up ") {
			stopped = append(stopped, name+": "+status)
		} else if len(expected) > 0 && !expected[name] {
			unmanaged = append(unmanaged, fmt.Sprintf("%s: %s, project=%s", name, status, project))
		}
	}
	var findings []doctorFinding
	if len(stopped) == 0 {
		findings = append(findings, findingOK("docker", "no stopped Docker containers detected"))
	} else {
		findings = append(findings, finding("warn", "docker", fmt.Sprintf("%d stopped Docker container(s) detected", len(stopped)), limitLines(stopped, 12)))
	}
	if len(unmanaged) > 0 {
		findings = append(findings, finding("info", "docker", fmt.Sprintf("%d running Docker container(s) are not declared in Plugroot", len(unmanaged)), limitLines(unmanaged, 12)))
	}
	return findings
}

func doctorDockerNetworks() doctorFinding {
	out := runCommand("docker", []string{"network", "ls", "--format", "{{.Name}}\t{{.Driver}}"}, "", nil)
	if out.Code != 0 {
		return finding("info", "docker-networks", "could not inspect Docker networks", out.Text)
	}
	var empty []string
	for _, line := range nonBlankLines(out.Text) {
		parts := strings.Split(line, "\t")
		name, driver := parts[0], ""
		if len(parts) > 1 {
			driver = parts[1]
		}
		if name == "bridge" || name == "host" || name == "none" || driver != "bridge" {
			continue
		}
		inspected := runCommand("docker", []string{"network", "inspect", "-f", "{{len .Containers}}", name}, "", nil)
		if inspected.Code == 0 && strings.TrimSpace(inspected.Text) == "0" {
			empty = append(empty, name)
		}
	}
	if len(empty) == 0 {
		return findingOK("docker-networks", "no empty custom Docker networks detected")
	}
	return finding("warn", "docker-networks", fmt.Sprintf("%d empty custom Docker network(s) detected", len(empty)), limitLines(empty, 12))
}

func doctorDockerVolumes() doctorFinding {
	out := runCommand("docker", []string{"volume", "ls", "-qf", "dangling=true"}, "", nil)
	if out.Code != 0 {
		return finding("info", "docker-volumes", "could not inspect unused Docker volumes", out.Text)
	}
	volumes := nonBlankLines(out.Text)
	if len(volumes) == 0 {
		return findingOK("docker-volumes", "no unused Docker volumes detected")
	}
	return finding("warn", "docker-volumes", fmt.Sprintf("%d unused Docker volume(s) detected", len(volumes)), limitLines(volumes, 12))
}

func doctorDockerReclaimable() doctorFinding {
	out := runCommand("docker", []string{"system", "df", "--format", "{{json .}}"}, "", nil)
	if out.Code != 0 {
		return finding("info", "docker-cache", "could not inspect Docker reclaimable data", out.Text)
	}
	var reclaimable []string
	for _, line := range nonBlankLines(out.Text) {
		var value any
		if json.Unmarshal([]byte(line), &value) != nil {
			continue
		}
		object, _ := value.(map[string]any)
		itemType, ok := object["Type"].(string)
		if !ok {
			itemType = "Docker data"
		}
		amount, _ := object["Reclaimable"].(string)
		if !strings.HasPrefix(amount, "0B") {
			reclaimable = append(reclaimable, fmt.Sprintf("%s: %s reclaimable", itemType, amount))
		}
	}
	if len(reclaimable) == 0 {
		return findingOK("docker-cache", "Docker reports no reclaimable data")
	}
	return finding("info", "docker-cache", "Docker has reclaimable data", limitLines(reclaimable, 8))
}
