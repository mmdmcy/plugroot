package main

// boundary: the code root holds code only, and private state lives outside
// the Git checkout.

import (
	"fmt"
	"os"
)

type boundaryFinding struct {
	severity string // "error" or "warning"
	path     *string
	message  string
}

func boundaryError(path string, message string) boundaryFinding {
	return boundaryFinding{severity: "error", path: &path, message: message}
}

func boundaryWarning(path *string, message string) boundaryFinding {
	return boundaryFinding{severity: "warning", path: path, message: message}
}

const boundaryHelp = `Usage:
  plugroot boundary
  plugroot boundary --strict

Checks that the code root is code-only and private state lives outside the
Git checkout. Errors always fail. --strict also fails on warnings.
`

func cmdBoundary(ctx *Context, args []string) (int, error) {
	strict := false
	for _, arg := range args {
		switch arg {
		case "-h", "--help":
			fmt.Print(boundaryHelp + "\n")
			return 0, nil
		case "--strict":
			strict = true
		default:
			fmt.Fprintln(os.Stderr, "usage: plugroot boundary [--strict]")
			return 2, nil
		}
	}
	findings, err := boundaryFindings(ctx)
	if err != nil {
		return 0, err
	}
	codeRoot, stateRoot := cleanPath(ctx.Root), cleanPath(ctx.stateRoot())
	repoDir, hasRepoDir := ctx.repoDir()
	if len(findings) == 0 {
		fmt.Printf("code root: %s\n", codeRoot)
		fmt.Printf("state root: %s\n", stateRoot)
		if hasRepoDir {
			fmt.Printf("repo dir: %s\n", cleanPath(repoDir))
		}
		fmt.Println("boundary: ok")
		return 0, nil
	}
	errorsFound := 0
	for _, f := range findings {
		if f.severity == "error" {
			errorsFound++
		}
	}
	warnings := len(findings) - errorsFound
	fmt.Fprintf(os.Stderr, "code root: %s\n", codeRoot)
	fmt.Fprintf(os.Stderr, "state root: %s\n", stateRoot)
	if hasRepoDir {
		fmt.Fprintf(os.Stderr, "repo dir: %s\n", cleanPath(repoDir))
	}
	fmt.Fprintf(os.Stderr, "boundary: found %d error(s), %d warning(s)\n", errorsFound, warnings)
	for _, f := range findings {
		if f.path != nil {
			fmt.Fprintf(os.Stderr, "%s: %s: %s\n", f.severity, *f.path, f.message)
		} else {
			fmt.Fprintf(os.Stderr, "%s: %s\n", f.severity, f.message)
		}
	}
	if errorsFound > 0 || strict && warnings > 0 {
		return 1, nil
	}
	return 0, nil
}

func boundaryFindings(ctx *Context) ([]boundaryFinding, error) {
	var findings []boundaryFinding
	codeRoot, stateRoot := cleanPath(ctx.Root), cleanPath(ctx.stateRoot())
	if config := ctx.Manifest.Plugroot; config != nil {
		if config.CodeRoot != nil {
			configured := cleanPath(resolvePath(ctx.Root, *config.CodeRoot))
			if !samePath(configured, codeRoot) {
				findings = append(findings, boundaryWarning(&configured, "configured code_root differs from the active --root checkout"))
			}
		}
		if config.StateRoot == nil {
			if config.Root != nil {
				findings = append(findings, boundaryWarning(nil, "plugroot.root is legacy; use plugroot.state_root for private machine state"))
			} else {
				findings = append(findings, boundaryWarning(nil, "plugroot.state_root is not configured; private state falls back to the code root"))
			}
		}
	} else {
		findings = append(findings, boundaryWarning(nil, "missing [plugroot] section; private state falls back to the code root"))
	}
	if sameOrInside(codeRoot, stateRoot) {
		findings = append(findings, boundaryError(stateRoot, "state root must live outside the code checkout"))
	}
	if repoDir, ok := ctx.repoDir(); ok {
		repoDir = cleanPath(repoDir)
		if sameOrInside(codeRoot, repoDir) {
			findings = append(findings, boundaryError(repoDir, "repo_dir must live outside the Plugroot code checkout"))
		}
		if gitRoot, ok := nearestGitRoot(repoDir); ok && sameOrInside(gitRoot, repoDir) {
			findings = append(findings, boundaryError(repoDir, "repo_dir is inside a Git checkout; keep cloned app repos in private state"))
		}
	}
	if gitRoot, ok := nearestGitRoot(stateRoot); ok && sameOrInside(gitRoot, stateRoot) {
		findings = append(findings, boundaryError(stateRoot, "state root is inside a Git checkout"))
	}
	for _, link := range ctx.Manifest.Symlink {
		source := cleanPath(resolvePath(ctx.Root, link.Source))
		if !sameOrInside(stateRoot, source) {
			findings = append(findings, boundaryError(source, "managed symlink source must live inside the state root"))
		}
	}
	for _, rel := range []string{".env", "plugroot.local.toml", ".plugroot", "repos", "data", "media", "backups"} {
		if path := joinPath(ctx.Root, rel); exists(path) {
			findings = append(findings, boundaryError(path, "private runtime path exists inside the code checkout"))
		}
	}
	for _, path := range servicePrivatePaths(ctx.Root) {
		findings = append(findings, boundaryError(path, "private service state exists inside the code checkout"))
	}
	if audit, err := auditPublic(ctx.Root); err != nil {
		root := ctx.Root
		findings = append(findings, boundaryWarning(&root, fmt.Sprintf("could not run public audit: %s", err)))
	} else {
		for _, f := range audit {
			message := "public audit issue: " + f.message
			if f.line > 0 {
				message = fmt.Sprintf("public audit issue at line %d: %s", f.line, f.message)
			}
			findings = append(findings, boundaryError(joinPath(ctx.Root, f.path), message))
		}
	}
	if _, ok := nearestGitRoot(ctx.Root); !ok {
		root := ctx.Root
		findings = append(findings, boundaryWarning(&root, "code root is not inside a Git checkout; tracked-file auditing is limited"))
	}
	return findings, nil
}

func servicePrivatePaths(codeRoot string) []string {
	var paths []string
	f, err := os.Open(joinPath(codeRoot, "services"))
	if err != nil {
		return paths
	}
	entries, _ := f.ReadDir(-1)
	f.Close()
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		for _, name := range []string{".env", "data", "cache", "config", "secrets"} {
			if path := joinPath(joinPath(joinPath(codeRoot, "services"), entry.Name()), name); exists(path) {
				paths = append(paths, path)
			}
		}
	}
	return paths
}
