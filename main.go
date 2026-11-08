// Plugroot is a private-first selfhost harness: one manifest describes the
// Docker Compose stacks, Git checkouts, systemd units and private listeners
// of a local server, with a terminal and a small web dashboard.
//
// Go with the standard library only. It replaces the Rust version (kept in
// archive/rust) with the same commands, flags, output, files and HTTP
// behaviour.
package main

import (
	"fmt"
	"os"
)

// version is the component version the status endpoint reports.
const version = "0.1.0"

func main() {
	code, err := run(os.Args[1:])
	if err != nil {
		fmt.Fprintf(os.Stderr, "plugroot: %s\n", err)
		os.Exit(1)
	}
	os.Exit(code)
}

func run(argv []string) (int, error) {
	root, args, err := parseGlobalArgs(argv)
	if err != nil {
		return 0, err
	}
	command := "help"
	if len(args) > 0 {
		command = args[0]
	}
	switch command {
	case "help", "-h", "--help":
		printHelp()
		return 0, nil
	case "audit-public":
		return cmdAuditPublic(root, args[1:])
	case "restore":
		return cmdRestore(root, args[1:])
	}

	ctx, err := loadContext(root)
	if err != nil {
		return 0, err
	}
	rest := args[1:]
	switch command {
	case "boundary":
		return cmdBoundary(ctx, rest)
	case "status":
		return cmdStatus(ctx, rest)
	case "apply":
		return cmdApply(ctx, rest)
	case "repos":
		return cmdRepos(ctx, rest)
	case "doctor":
		return cmdDoctor(ctx, rest)
	case "release-check":
		return cmdReleaseCheck(ctx)
	case "release":
		return cmdRelease(ctx, rest)
	case "backup":
		return cmdBackup(ctx, rest)
	case "emergency":
		return cmdEmergency(ctx, rest)
	case "up":
		return cmdAction(ctx, "start", rest)
	case "down":
		return cmdAction(ctx, "stop", rest)
	case "restart":
		return cmdAction(ctx, "restart", rest)
	case "logs":
		return cmdAction(ctx, "logs", rest)
	case "tui":
		return cmdTUI(ctx, rest)
	case "web":
		return cmdWeb(ctx, rest)
	case "list":
		return cmdList(ctx)
	}
	fmt.Fprintf(os.Stderr, "unknown command: %s\n", command)
	printHelp()
	return 2, nil
}

// parseGlobalArgs takes --root PATH (or --root=PATH) from anywhere in the
// arguments; the code root defaults to the working directory.
func parseGlobalArgs(args []string) (string, []string, error) {
	root := ""
	rootGiven := false
	var rest []string
	for i := 0; i < len(args); i++ {
		switch arg := args[i]; {
		case arg == "--root":
			if i+1 >= len(args) {
				return "", nil, fmt.Errorf("--root requires a path")
			}
			root, rootGiven = args[i+1], true
			i++
		case len(arg) >= 7 && arg[:7] == "--root=":
			root, rootGiven = trimAllPrefix(arg, "--root="), true
		default:
			rest = append(rest, arg)
		}
	}
	if !rootGiven {
		cwd, err := os.Getwd()
		if err != nil {
			return "", nil, ioError(err)
		}
		root = cwd
	}
	return root, rest, nil
}

// trimAllPrefix removes prefix as often as it repeats, as Rust's
// trim_start_matches did.
func trimAllPrefix(s, prefix string) string {
	for prefix != "" && len(s) >= len(prefix) && s[:len(prefix)] == prefix {
		s = s[len(prefix):]
	}
	return s
}

func printHelp() {
	fmt.Print(helpText + "\n")
}

const helpText = `Plugroot - private-first selfhost harness

Usage:
  plugroot [--root <path>] status [--json]
  plugroot [--root <path>] list
  plugroot [--root <path>] apply [--dry-run] [--sync-repos]
  plugroot [--root <path>] repos sync
  plugroot [--root <path>] doctor [--json] [--strict]
  plugroot [--root <path>] release-check
  plugroot [--root <path>] release <service> [<binary>|rollback] [--restart]
  plugroot [--root <path>] backup [check] [directory]
  plugroot restore [check|finish] [directory-or-archive]
  plugroot [--root <path>] emergency export --output <directory> --yes
  plugroot [--root <path>] up|down|restart|logs <service|all>
  plugroot [--root <path>] tui [--once]
  plugroot [--root <path>] web [--bind <addr:port>]...
  plugroot [--root <path>] boundary [--strict]
  plugroot [--root <path>] audit-public [--install-hook]

Files:
  plugroot.toml                         public manifest in the code root
  $PLUGROOT_STATE_ROOT/.env             private local values
  $PLUGROOT_STATE_ROOT/plugroot.local.toml  private local overlay
`
