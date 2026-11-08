package main

// The dashboard's HTML.

import (
	"fmt"
	"sort"
	"strings"
)

// renderWebLogin is the sign-in page; message is shown when not "".
func renderWebLogin(ctx *Context, message string) string {
	user := ctx.EnvValues["PLUGROOT_WEB_USER"]
	if user == "" {
		user = "plugroot"
	}
	errorHTML := ""
	if message != "" {
		errorHTML = `<p class="error">` + htmlEscape(message) + `</p>`
	}
	return strings.NewReplacer("{{ERROR}}", errorHTML, "{{USER}}", htmlEscape(user)).Replace(loginTemplate)
}

func renderWeb(ctx *Context) string {
	rows := statusRows(ctx)
	total, online, attention, idle := len(rows), 0, 0, 0
	for _, row := range rows {
		if row.State == "online" {
			online++
		}
		if webNeedsAttention(row) {
			attention++
		} else if row.Optional && row.State != "online" {
			idle++
		}
	}
	hostName := deref(ctx.hostField(func(h *HostConfig) *string { return h.Name }), "plugroot-host")
	privateIP := deref(ctx.hostField(func(h *HostConfig) *string { return h.PrivateIP }), "127.0.0.1")
	return strings.NewReplacer(
		"{{HOST}}", htmlEscape(hostName),
		"{{TOTAL}}", fmt.Sprint(total),
		"{{PRIVATE_IP}}", htmlEscape(privateIP),
		"{{STATE_ROOT}}", htmlEscape(cleanPath(ctx.stateRoot())),
		"{{ONLINE}}", fmt.Sprint(online),
		"{{ATTENTION}}", fmt.Sprint(attention),
		"{{IDLE}}", fmt.Sprint(idle),
		"{{SERVICES}}", renderWebServiceLists(rows),
	).Replace(pageTemplate)
}

func renderWebServiceLists(rows []StatusRow) string {
	if len(rows) == 0 {
		return `<section class="empty"><h2>No Services</h2><p>Add service entries to plugroot.toml or the private overlay to populate this overview.</p></section>`
	}
	var launchable, unlinked []StatusRow
	for _, row := range rows {
		if row.URL != nil {
			launchable = append(launchable, row)
		} else {
			unlinked = append(unlinked, row)
		}
	}
	sortRows := func(list []StatusRow) {
		sort.SliceStable(list, func(a, b int) bool {
			na, nb := webNeedsAttention(list[a]), webNeedsAttention(list[b])
			if na != nb {
				return na
			}
			return list[a].Name < list[b].Name
		})
	}
	sortRows(launchable)
	sortRows(unlinked)
	cards := func(list []StatusRow) string {
		var b strings.Builder
		for _, row := range list {
			b.WriteString(renderWebService(row))
		}
		return b.String()
	}
	out := ""
	if len(launchable) > 0 {
		out += `<section class="services"><div class="list-head"><h2>Services</h2><p>Tap a service name or Open to launch it.</p></div><div class="grid">` + cards(launchable) + `</div></section>`
	}
	if len(unlinked) > 0 {
		out += `<section class="services unlinked"><div class="list-head"><h2>No direct link</h2><p>Status-only services</p></div><div class="grid">` + cards(unlinked) + `</div></section>`
	}
	return out
}

func renderWebService(row StatusRow) string {
	var actions strings.Builder
	for _, action := range row.Controls {
		fmt.Fprintf(&actions, `<form method="post" action="/api/action/%s/%s"><button>%s</button></form>`,
			htmlEscape(row.ID), htmlEscape(action), htmlEscape(action))
	}
	actionHTML := actions.String()
	if actionHTML == "" {
		actionHTML = `<span class="chip disabled">No controls</span>`
	}
	var title, launch string
	if row.URL != nil {
		title = fmt.Sprintf(`<a class="svc-title svc-direct" href="%s"><h3>%s</h3><p>%s</p></a>`,
			htmlEscape(*row.URL), htmlEscape(row.Name), htmlEscape(row.Detail))
		launch = fmt.Sprintf(`<a class="launch" href="%s" aria-label="Open %s">Open</a>`, htmlEscape(*row.URL), htmlEscape(row.Name))
	} else {
		title = fmt.Sprintf(`<div class="svc-title"><h3>%s</h3><p>%s</p></div>`, htmlEscape(row.Name), htmlEscape(row.Detail))
		launch = `<span class="no-launch">No link</span>`
	}
	description := ""
	if row.Description != nil {
		description = `<p class="desc">` + htmlEscape(*row.Description) + `</p>`
	}
	ports := ""
	if len(row.Ports) > 0 {
		ports = `<span class="chip">` + htmlEscape(strings.Join(row.Ports, ", ")) + `</span>`
	}
	repo := ""
	if row.Repo != nil {
		repo = `<span class="chip">repo ` + htmlEscape(*row.Repo) + `</span>`
	}
	access := ""
	if row.Access != nil {
		access = `<span class="chip">access ` + htmlEscape(*row.Access) + `</span>`
	}
	optional := ""
	if row.Optional {
		optional = `<span class="chip">optional</span>`
	}
	return `<article class="svc"><div class="svc-row">` + title + `<span class="state ` + webStateClass(row) + `">` +
		htmlEscape(row.State) + `</span>` + launch + `</div><details class="svc-more"><summary>Details</summary><div class="svc-body"><p class="detail">` +
		htmlEscape(row.Detail) + `</p>` + description + `<div class="svc-meta"><span class="chip">id ` + htmlEscape(row.ID) +
		`</span><span class="chip">` + htmlEscape(row.Kind) + `</span>` + repo + access + ports + optional +
		`</div><div class="actions">` + actionHTML + `</div></div></details></article>`
}

func webNeedsAttention(row StatusRow) bool {
	return row.State != "online" && !(row.Optional && (row.State == "offline" || row.State == "missing"))
}

func webStateClass(row StatusRow) string {
	switch {
	case row.State == "online":
		return "state-online"
	case !webNeedsAttention(row):
		return "state-idle"
	case row.State == "offline" || row.State == "missing":
		return "state-attention"
	}
	return "state-unknown"
}

func htmlEscape(value string) string {
	return strings.NewReplacer("&", "&amp;", "<", "&lt;", ">", "&gt;", `"`, "&quot;").Replace(value)
}
