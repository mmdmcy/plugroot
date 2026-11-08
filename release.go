package main

// Binary releases for systemd-backed services.
//
// A service that runs a single binary keeps its builds under
// <state root>/releases/<service>/<stamp>/<service> with a `current`
// symlink that the unit's ExecStart points at. Installing a build copies it
// into a fresh stamped directory and flips `current` with one rename, so the
// previous build stays on disk and `rollback` is the same flip the other way.

import (
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"time"
)

const currentRelease = "current"

// releaseChange is what `current` pointed at after a change, and before.
type releaseChange struct {
	current  string
	previous string // "" when there was none
}

type releaseListing struct {
	current  string // "" when there is none
	releases []string
}

func cmdRelease(ctx *Context, args []string) (int, error) {
	restart := false
	var words []string
	for _, arg := range args {
		switch {
		case arg == "--restart":
			restart = true
		case strings.HasPrefix(arg, "--"):
			fmt.Fprintf(os.Stderr, "unknown option: %s\n", arg)
			return 2, nil
		default:
			words = append(words, arg)
		}
	}
	if len(words) == 0 {
		fmt.Fprintln(os.Stderr, "usage: plugroot release <service> [<binary>|rollback] [--restart]")
		return 2, nil
	}
	target := words[0]
	s, ok := ctx.service(target)
	if !ok {
		fmt.Fprintf(os.Stderr, "unknown service: %s\n", target)
		return 2, nil
	}
	if s.Kind != "systemd" && s.Kind != "user-systemd" {
		fmt.Fprintf(os.Stderr, "%s is a %s service; releases are for systemd units that run releases/%s/current\n", s.ID, s.Kind, s.ID)
		return 2, nil
	}
	dir := releaseDir(ctx, s)
	rollingBack := len(words) > 1 && words[1] == "rollback"
	var change releaseChange
	var err error
	switch {
	case len(words) == 1:
		listing, err := listReleases(dir)
		if err != nil {
			return 0, err
		}
		fmt.Printf("%s: %s\n", s.ID, dir)
		if len(listing.releases) == 0 {
			fmt.Println("  no releases installed")
		}
		for _, release := range listing.releases {
			marker := ""
			if listing.current == release {
				marker = " <- current"
			}
			fmt.Printf("  %s%s\n", release, marker)
		}
		return 0, nil
	case rollingBack:
		change, err = rollbackRelease(dir)
	default:
		stamp := uniqueStamp(dir, releaseStamp())
		change, err = installRelease(dir, s.ID, words[1], stamp)
	}
	if err != nil {
		return 0, err
	}
	fmt.Printf("%s: current -> %s\n", s.ID, change.current)
	switch {
	case change.previous != "" && !rollingBack:
		fmt.Printf("kept %s; `plugroot release %s rollback` goes back to it\n", change.previous, s.ID)
	case change.previous != "":
		fmt.Printf("left %s on disk\n", change.previous)
	default:
		fmt.Println("no earlier build to roll back to")
	}
	if !restart {
		fmt.Printf("the running service still uses the old build; `plugroot restart %s` switches\n", s.ID)
		return 0, nil
	}
	if !hasControl(s, "restart") {
		fmt.Fprintf(os.Stderr, "%s does not allow `restart`\n", s.ID)
		return 2, nil
	}
	fmt.Printf("==> %s restart\n", s.ID)
	out := runFixedAction(ctx, s, "restart")
	printCmdOutput(out)
	return min(out.Code, 255), nil
}

// uniqueStamp is stamp, or stamp-2, stamp-3, ... when a build landed in the
// same second.
func uniqueStamp(dir, stamp string) string {
	if !exists(joinPath(dir, stamp)) {
		return stamp
	}
	for n := 2; ; n++ {
		candidate := fmt.Sprintf("%s-%d", stamp, n)
		if !exists(joinPath(dir, candidate)) {
			return candidate
		}
	}
}

func releaseDir(ctx *Context, s Service) string {
	return joinPath(joinPath(ctx.stateRoot(), "releases"), s.ID)
}

// installRelease copies binary to <dir>/<stamp>/<name> (mode 0755) and
// points <dir>/current at the new directory. Nothing on disk is removed.
func installRelease(dir, name, binary, stamp string) (releaseChange, error) {
	info, err := os.Stat(binary)
	if err != nil {
		return releaseChange{}, fmt.Errorf("cannot read %s: %s", binary, ioError(err))
	}
	if !info.Mode().IsRegular() {
		return releaseChange{}, fmt.Errorf("%s is not a file", binary)
	}
	if stamp == "" || strings.HasPrefix(stamp, ".") || stamp == currentRelease || strings.Contains(stamp, "/") {
		return releaseChange{}, fmt.Errorf("invalid release name %s", rustDebugString(stamp))
	}
	if err := os.MkdirAll(dir, 0o777); err != nil {
		return releaseChange{}, writableHint(dir, err)
	}
	stage := joinPath(dir, stamp)
	if exists(stage) {
		return releaseChange{}, fmt.Errorf("%s already exists; try again in a second", stage)
	}
	if err := os.Mkdir(stage, 0o777); err != nil {
		return releaseChange{}, writableHint(dir, err)
	}
	installed := joinPath(stage, name)
	if err := copyFile(binary, installed); err != nil {
		return releaseChange{}, err
	}
	if err := os.Chmod(installed, 0o755); err != nil {
		return releaseChange{}, ioError(err)
	}
	previous := currentReleaseName(dir)
	if err := pointCurrent(dir, stamp); err != nil {
		return releaseChange{}, err
	}
	return releaseChange{current: stamp, previous: previous}, nil
}

// rollbackRelease points current at the newest release older than it.
func rollbackRelease(dir string) (releaseChange, error) {
	listing, err := listReleases(dir)
	if err != nil {
		return releaseChange{}, err
	}
	if listing.current == "" {
		return releaseChange{}, fmt.Errorf("%s has no current release", dir)
	}
	target := ""
	for i := len(listing.releases) - 1; i >= 0; i-- {
		if listing.releases[i] < listing.current {
			target = listing.releases[i]
			break
		}
	}
	if target == "" {
		return releaseChange{}, fmt.Errorf("no release older than %s in %s", listing.current, dir)
	}
	if err := pointCurrent(dir, target); err != nil {
		return releaseChange{}, err
	}
	return releaseChange{current: target, previous: listing.current}, nil
}

func listReleases(dir string) (releaseListing, error) {
	releases := []string{}
	f, err := os.Open(dir)
	switch {
	case err == nil:
		entries, err := f.ReadDir(-1)
		f.Close()
		if err != nil {
			return releaseListing{}, ioError(err)
		}
		for _, entry := range entries {
			name := entry.Name()
			if name == currentRelease || strings.HasPrefix(name, ".") {
				continue
			}
			if entry.IsDir() {
				releases = append(releases, name)
			}
		}
	case errors.Is(err, os.ErrNotExist):
	default:
		return releaseListing{}, ioError(err)
	}
	sort.Strings(releases)
	return releaseListing{current: currentReleaseName(dir), releases: releases}, nil
}

func currentReleaseName(dir string) string {
	target, err := os.Readlink(joinPath(dir, currentRelease))
	if err != nil {
		return ""
	}
	name, _ := fileName(target)
	return name
}

// pointCurrent replaces current atomically: a temporary symlink is renamed
// over it.
func pointCurrent(dir, release string) error {
	staging := joinPath(dir, fmt.Sprintf(".%s.%s.%d", currentRelease, release, os.Getpid()))
	os.Remove(staging)
	if err := os.Symlink(release, staging); err != nil {
		return writableHint(dir, err)
	}
	if err := os.Rename(staging, joinPath(dir, currentRelease)); err != nil {
		os.Remove(staging)
		return writableHint(dir, err)
	}
	return nil
}

func writableHint(dir string, err error) error {
	if errors.Is(err, os.ErrPermission) {
		return fmt.Errorf("%s is not writable by this user: run plugroot with sudo, or give the directory to the service user", dir)
	}
	return ioError(err)
}

// releaseStamp is UTC YYYYMMDDHHMMSS, the naming the releases directory
// already uses.
func releaseStamp() string { return stampFromUnix(time.Now().Unix()) }

func stampFromUnix(secs int64) string {
	if secs < 0 {
		secs = 0
	}
	return time.Unix(secs, 0).UTC().Format("20060102150405")
}

// rustDebugString quotes like Rust's {:?} for a string.
func rustDebugString(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for _, r := range s {
		switch r {
		case '"':
			b.WriteString(`\"`)
		case '\\':
			b.WriteString(`\\`)
		case '\n':
			b.WriteString(`\n`)
		case '\r':
			b.WriteString(`\r`)
		case '\t':
			b.WriteString(`\t`)
		case 0:
			b.WriteString(`\0`)
		default:
			if r < 0x20 || r == 0x7f {
				fmt.Fprintf(&b, `\u{%x}`, r)
			} else {
				b.WriteRune(r)
			}
		}
	}
	b.WriteByte('"')
	return b.String()
}
