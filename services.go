package main

// Start, stop, restart and logs for a declared service.

import (
	"fmt"
	"os"
)

func cmdAction(ctx *Context, action string, args []string) (int, error) {
	if len(args) == 0 {
		fmt.Fprintln(os.Stderr, "missing service target")
		return 2, nil
	}
	target := args[0]
	services := ctx.servicesForTarget(target)
	if len(services) == 0 {
		fmt.Fprintf(os.Stderr, "unknown service: %s\n", target)
		return 2, nil
	}
	final := 0
	for _, s := range services {
		if !hasControl(s, action) {
			fmt.Fprintf(os.Stderr, "%s does not allow `%s`\n", s.ID, action)
			final = 2
			continue
		}
		fmt.Printf("==> %s %s\n", s.ID, action)
		var out CmdOutput
		if action == "logs" {
			out = logsFor(ctx, s)
		} else {
			out = runFixedAction(ctx, s, action)
		}
		printCmdOutput(out)
		if out.Code != 0 {
			final = min(out.Code, 255)
		}
	}
	return final, nil
}

func hasControl(s Service, action string) bool {
	for _, control := range s.Controls {
		if control == action {
			return true
		}
	}
	return false
}

// composeArgs is `docker compose` for the service's directory and its
// compose.yaml.
func composeArgs(ctx *Context, s Service) ([]string, bool) {
	if s.Path == nil {
		return nil, false
	}
	dir := resolvePath(ctx.Root, *s.Path)
	return []string{"compose", "--project-directory", dir, "-f", joinPath(dir, "compose.yaml")}, true
}

// composeEnvValues is every environment value plus the service's env_file,
// passed to docker compose through its environment, never as --env-file.
func composeEnvValues(ctx *Context, s Service) map[string]string {
	values := make(map[string]string, len(ctx.EnvValues))
	for key, value := range ctx.EnvValues {
		values[key] = value
	}
	if s.EnvFile != nil {
		for key, value := range loadEnvFile(resolvePath(ctx.Root, *s.EnvFile)) {
			values[key] = value
		}
	}
	return values
}

func logsFor(ctx *Context, s Service) CmdOutput {
	switch s.Kind {
	case "compose":
		args, ok := composeArgs(ctx, s)
		if !ok {
			return CmdOutput{Code: 2, Text: "compose service is missing path"}
		}
		args = append(args, "logs", "--tail", "200", "--no-color")
		return runCommandWithEnvValues("docker", args, "", composeEnvValues(ctx, s))
	case "systemd":
		return runCommand("journalctl", []string{"-u", deref(s.Unit, ""), "-n", "200", "--no-pager"}, "", nil)
	case "user-systemd":
		prefix, ok := userSystemdPrefix(deref(s.User, ""))
		if !ok {
			return CmdOutput{Code: 2, Text: "user not found"}
		}
		return runPrefixedCommand(prefix, "journalctl", []string{"--user", "-u", deref(s.Unit, ""), "-n", "200", "--no-pager"})
	}
	return CmdOutput{Code: 2, Text: "logs unavailable for this service type"}
}

func runFixedAction(ctx *Context, s Service, action string) CmdOutput {
	switch s.Kind {
	case "compose":
		args, ok := composeArgs(ctx, s)
		if !ok {
			return CmdOutput{Code: 2, Text: "compose service is missing path"}
		}
		switch action {
		case "start":
			args = append(args, "up", "-d")
		case "stop":
			args = append(args, "down")
		case "restart":
			args = append(args, "restart")
		default:
			return CmdOutput{Code: 2, Text: "unsupported action " + action}
		}
		return runCommandWithEnvValues("docker", args, "", composeEnvValues(ctx, s))
	case "systemd":
		switch action {
		case "start", "stop", "restart":
		default:
			return CmdOutput{Code: 2, Text: "unsupported action " + action}
		}
		return runCommand("systemctl", []string{action, deref(s.Unit, "")}, "", nil)
	case "user-systemd":
		prefix, ok := userSystemdPrefix(deref(s.User, ""))
		if !ok {
			return CmdOutput{Code: 2, Text: "user not found"}
		}
		switch action {
		case "start", "stop", "restart":
		default:
			return CmdOutput{Code: 2, Text: "unsupported action " + action}
		}
		return runPrefixedCommand(prefix, "systemctl", []string{"--user", action, deref(s.Unit, "")})
	}
	return CmdOutput{Code: 2, Text: "this service type has no fixed action"}
}
