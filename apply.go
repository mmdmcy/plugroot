package main

// apply: managed directories, files, repositories, symlinks and generated
// systemd units; and `repos sync`.

import (
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type applyOptions struct {
	dryRun    bool
	syncRepos bool
}

const applyUsage = "usage: plugroot apply [--dry-run] [--sync-repos]"

func parseApplyOptions(args []string) (applyOptions, error) {
	var options applyOptions
	for _, arg := range args {
		switch arg {
		case "--dry-run":
			options.dryRun = true
		case "--sync-repos":
			options.syncRepos = true
		case "-h", "--help":
			return options, errors.New(applyUsage)
		default:
			return options, fmt.Errorf("unknown apply option: %s", arg)
		}
	}
	return options, nil
}

func pick(dryRun bool, would, doing string) string {
	if dryRun {
		return would
	}
	return doing
}

func cmdApply(ctx *Context, args []string) (int, error) {
	options, err := parseApplyOptions(args)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		fmt.Fprintln(os.Stderr, applyUsage)
		return 2, nil
	}
	dryRun := options.dryRun
	stateRoot := ctx.stateRoot()
	generatedRoot := joinPath(stateRoot, ".plugroot/generated")
	fmt.Printf("code root: %s\n", ctx.Root)
	fmt.Printf("state root: %s\n", stateRoot)
	if repoDir, ok := ctx.repoDir(); ok {
		fmt.Printf("repo dir: %s\n", repoDir)
	}
	if dryRun {
		fmt.Println("dry run: no files or services will be changed")
	}
	if options.syncRepos {
		fmt.Println("repo sync: enabled")
	} else {
		fmt.Println("repo sync: skipped; use `plugroot repos sync` or `plugroot apply --sync-repos`")
	}
	if !dryRun {
		if err := os.MkdirAll(generatedRoot, 0o777); err != nil {
			return 0, ioError(err)
		}
	}
	for _, d := range ctx.Manifest.Directory {
		fmt.Printf("%s directory %s\n", pick(dryRun, "would create", "creating"), d.Path)
		if !dryRun {
			if err := applyDirectory(d); err != nil {
				return 0, err
			}
		}
	}
	for _, f := range ctx.Manifest.File {
		fmt.Printf("%s file %s -> %s\n", pick(dryRun, "would copy", "copying"), f.Source, f.Target)
		if !dryRun {
			if err := applyFile(ctx, f); err != nil {
				return 0, err
			}
		}
	}
	for _, r := range ctx.Manifest.Repo {
		if options.syncRepos {
			fmt.Printf("%s repo %s -> %s\n", pick(dryRun, "would sync", "syncing"), r.ID, r.Path)
			if !dryRun {
				if err := syncRepo(r); err != nil {
					return 0, err
				}
			}
		} else {
			fmt.Printf("skipping repo sync %s -> %s\n", r.ID, r.Path)
		}
	}
	for _, link := range ctx.Manifest.Symlink {
		fmt.Printf("%s symlink %s -> %s\n", pick(dryRun, "would create", "creating"), link.Target, link.Source)
		if !dryRun {
			if err := applySymlink(ctx, link); err != nil {
				return 0, err
			}
		}
	}
	for _, s := range ctx.Manifest.Service {
		unit, ok, err := generateUnit(ctx, s)
		if err != nil {
			return 0, err
		}
		if !ok {
			continue
		}
		scope := "system"
		if s.Kind == "user-systemd" {
			scope = "user"
		}
		unitName := deref(s.Unit, "missing.service")
		generatedPath := joinPath(joinPath(joinPath(generatedRoot, "systemd"), scope), unitName)
		fmt.Printf("%s %s unit %s\n", pick(dryRun, "would write", "writing"), scope, unitName)
		if !dryRun {
			if parent, ok := parentPath(generatedPath); ok {
				if err := os.MkdirAll(parent, 0o777); err != nil {
					return 0, ioError(err)
				}
			}
			if err := os.WriteFile(generatedPath, []byte(unit), 0o666); err != nil {
				return 0, ioError(err)
			}
			if err := installUnit(s, generatedPath); err != nil {
				return 0, err
			}
		}
	}
	return 0, nil
}

func cmdRepos(ctx *Context, args []string) (int, error) {
	if len(args) > 0 && args[0] == "sync" {
		for _, r := range ctx.Manifest.Repo {
			fmt.Printf("syncing %s -> %s\n", r.ID, r.Path)
			if err := syncRepo(r); err != nil {
				return 0, err
			}
		}
		return 0, nil
	}
	fmt.Fprintln(os.Stderr, "usage: plugroot repos sync")
	return 2, nil
}

// gitStep runs one git command, prints its output and fails with message.
func gitStep(message string, args ...string) error {
	out := runCommand("git", args, "", nil)
	printCmdOutput(out)
	if out.Code != 0 {
		return errors.New(message)
	}
	return nil
}

func syncRepo(r Repo) error {
	if exists(joinPath(r.Path, ".git")) {
		if err := gitStep("git fetch failed", "-C", r.Path, "fetch", "--all", "--prune"); err != nil {
			return err
		}
		if r.GitRef != nil {
			if err := gitStep("git checkout failed", "-C", r.Path, "checkout", *r.GitRef); err != nil {
				return err
			}
			if err := gitStep("git pull failed", "-C", r.Path, "pull", "--ff-only"); err != nil {
				return err
			}
		}
		return nil
	}
	if parent, ok := parentPath(r.Path); ok {
		if err := os.MkdirAll(parent, 0o777); err != nil {
			return ioError(err)
		}
	}
	if err := gitStep("git clone failed", "clone", r.URL, r.Path); err != nil {
		return err
	}
	if r.GitRef != nil {
		return gitStep("git checkout failed", "-C", r.Path, "checkout", *r.GitRef)
	}
	return nil
}

func applyDirectory(d ManagedDir) error {
	if err := os.MkdirAll(d.Path, 0o777); err != nil {
		return ioError(err)
	}
	return applyOwnerMode(d.Path, d.Owner, d.Mode)
}

func applyFile(ctx *Context, f ManagedFile) error {
	source := resolvePath(ctx.Root, f.Source)
	if parent, ok := parentPath(f.Target); ok {
		if err := os.MkdirAll(parent, 0o777); err != nil {
			return ioError(err)
		}
	}
	if err := copyFile(source, f.Target); err != nil {
		return err
	}
	return applyOwnerMode(f.Target, f.Owner, f.Mode)
}

// copyFile is Rust's fs::copy: the contents and the permission bits.
func copyFile(source, target string) error {
	in, err := os.Open(source)
	if err != nil {
		return ioError(err)
	}
	defer in.Close()
	info, err := in.Stat()
	if err != nil {
		return ioError(err)
	}
	if !info.Mode().IsRegular() {
		return errors.New("the source path is neither a regular file nor a symlink to a regular file")
	}
	out, err := os.OpenFile(target, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, info.Mode().Perm())
	if err != nil {
		return ioError(err)
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return ioError(err)
	}
	if err := out.Chmod(info.Mode().Perm()); err != nil {
		out.Close()
		return ioError(err)
	}
	return ioError(out.Close())
}

func applySymlink(ctx *Context, link ManagedSymlink) error {
	stateRoot, err := canonicalize(ctx.stateRoot())
	if err != nil {
		return fmt.Errorf("canonicalize state root %s: %s", ctx.stateRoot(), err)
	}
	requested := resolvePath(ctx.Root, link.Source)
	source, err := canonicalize(requested)
	if err != nil {
		return fmt.Errorf("canonicalize symlink source %s: %s", requested, err)
	}
	if !sameOrInside(stateRoot, source) {
		return kindError{errInvalidInput, fmt.Sprintf("symlink source %s must be inside state root %s", source, stateRoot)}
	}
	target := resolvePath(ctx.Root, link.Target)
	parent, ok := parentPath(target)
	if !ok {
		return kindError{errInvalidInput, fmt.Sprintf("symlink target %s has no parent", target)}
	}
	if !isDir(parent) {
		return kindError{errNotFound, fmt.Sprintf("symlink target parent %s does not exist; create or sync it first", parent)}
	}
	if gitRoot, ok := nearestGitRoot(parent); ok {
		relative, ok := stripPrefixPath(target, gitRoot)
		if !ok {
			return kindError{errInvalidInput, fmt.Sprintf("symlink target %s could not be resolved inside Git checkout %s", target, gitRoot)}
		}
		cmd := exec.Command("git", "-C", gitRoot, "check-ignore", "--quiet", "--", relative)
		cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
		if err := cmd.Run(); err != nil {
			var exit *exec.ExitError
			if !errors.As(err, &exit) {
				return ioError(err)
			}
			return kindError{errInvalidInput, fmt.Sprintf("symlink target %s is inside a Git checkout but is not ignored", target)}
		}
	}
	info, err := os.Lstat(target)
	switch {
	case err == nil && info.Mode()&os.ModeSymlink != 0:
		existing, err := os.Readlink(target)
		if err != nil {
			return ioError(err)
		}
		if !isAbs(existing) {
			existing = joinPath(parent, existing)
		}
		if resolved, err := canonicalize(existing); err == nil && resolved == source {
			return nil
		}
		return kindError{errAlreadyExists, fmt.Sprintf("refusing to replace conflicting symlink %s", target)}
	case err == nil:
		return kindError{errAlreadyExists, fmt.Sprintf("refusing to replace existing path %s", target)}
	case errors.Is(err, os.ErrNotExist):
		return ioError(os.Symlink(source, target))
	}
	return ioError(err)
}

// canonicalize is Rust's fs::canonicalize: the absolute path with every
// link resolved; the error is worded as Rust's.
func canonicalize(path string) (string, error) {
	if !isAbs(path) {
		cwd, err := os.Getwd()
		if err != nil {
			return "", ioError(err)
		}
		path = joinPath(cwd, path)
	}
	resolved, err := filepath.EvalSymlinks(path)
	if err != nil {
		return "", ioError(err)
	}
	return resolved, nil
}

// errKind is the kind of an error the tests check, as Rust's ErrorKind.
type errKind int

const (
	errInvalidInput errKind = iota + 1
	errNotFound
	errAlreadyExists
	errPermissionDenied
)

type kindError struct {
	kind    errKind
	message string
}

func (e kindError) Error() string { return e.message }

func applyOwnerMode(path string, owner, mode *string) error {
	if mode != nil {
		if err := ensureSuccess(runCommand("chmod", []string{*mode, path}, "", nil)); err != nil {
			return err
		}
	}
	if owner != nil {
		if err := ensureSuccess(runCommand("chown", []string{*owner, path}, "", nil)); err != nil {
			return err
		}
	}
	return nil
}

// generateUnit is the unit file for a service: its unit_source, or one
// written from its command. ok is false when there is none.
func generateUnit(ctx *Context, s Service) (string, bool, error) {
	if s.Unit == nil {
		return "", false, nil
	}
	if s.UnitSource != nil {
		text, err := readText(resolvePath(ctx.Root, *s.UnitSource))
		if err != nil {
			return "", false, err
		}
		return text, true, nil
	}
	if s.Command == nil {
		return "", false, nil
	}
	workingDir := "."
	if s.WorkingDir != nil {
		workingDir = *s.WorkingDir
	} else if s.Path != nil {
		workingDir = *s.Path
	}
	var b strings.Builder
	b.WriteString("[Unit]\n")
	b.WriteString("Description=Plugroot - " + s.Name + "\n")
	b.WriteString("\n[Service]\n")
	b.WriteString("Type=simple\n")
	b.WriteString("WorkingDirectory=" + systemdQuote(workingDir) + "\n")
	for _, item := range s.Env {
		b.WriteString("Environment=" + systemdQuote(item) + "\n")
	}
	quoted := make([]string, len(*s.Command))
	for i, arg := range *s.Command {
		quoted[i] = systemdQuote(arg)
	}
	b.WriteString("ExecStart=" + strings.Join(quoted, " ") + "\n")
	b.WriteString("Restart=on-failure\n")
	b.WriteString("RestartSec=2\n")
	b.WriteString("\n[Install]\n")
	if s.Kind == "user-systemd" {
		b.WriteString("WantedBy=default.target\n")
	} else {
		b.WriteString("WantedBy=multi-user.target\n")
	}
	if *s.Unit == "" {
		return "", false, nil
	}
	return b.String(), true, nil
}

func systemdQuote(value string) string {
	plain := true
	for _, r := range value {
		if !(r < 0x80 && (r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || strings.ContainsRune("/:._-=@", r))) {
			plain = false
			break
		}
	}
	if plain {
		return value
	}
	return `"` + strings.ReplaceAll(strings.ReplaceAll(value, `\`, `\\`), `"`, `\"`) + `"`
}

func installUnit(s Service, generatedPath string) error {
	if s.Unit == nil {
		return kindError{errInvalidInput, "missing unit"}
	}
	unit := *s.Unit
	switch s.Kind {
	case "user-systemd":
		user := deref(s.User, "")
		home, ok := userHome(user)
		if !ok {
			return kindError{errInvalidInput, fmt.Sprintf("user %s not found", user)}
		}
		target := joinPath(joinPath(home, ".config/systemd/user"), unit)
		if parent, ok := parentPath(target); ok {
			if err := os.MkdirAll(parent, 0o777); err != nil {
				return ioError(err)
			}
		}
		if err := copyFile(generatedPath, target); err != nil {
			return err
		}
		prefix, ok := userSystemdPrefix(user)
		if !ok {
			return kindError{errInvalidInput, fmt.Sprintf("user %s not found", user)}
		}
		if err := ensureSuccess(runPrefixedCommand(prefix, "systemctl", []string{"--user", "daemon-reload"})); err != nil {
			return err
		}
		return ensureSuccess(runPrefixedCommand(prefix, "systemctl", []string{"--user", "enable", unit}))
	case "systemd":
		if !isRoot() {
			return kindError{errPermissionDenied, "system units require root"}
		}
		if err := copyFile(generatedPath, joinPath("/etc/systemd/system", unit)); err != nil {
			return err
		}
		if err := ensureSuccess(runCommand("systemctl", []string{"daemon-reload"}, "", nil)); err != nil {
			return err
		}
		return ensureSuccess(runCommand("systemctl", []string{"enable", unit}, "", nil))
	}
	return nil
}
