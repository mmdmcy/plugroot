package main

// restore: review a backup, extract the Plugroot roots onto a fresh host,
// and finish with apply, up and doctor.

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"os"
	"sort"
	"strings"
	"time"
)

type restoreArchiveKind int

const (
	restoreEncrypted restoreArchiveKind = iota + 1
	restorePlain
)

func (k restoreArchiveKind) label() string {
	if k == restoreEncrypted {
		return "encrypted GPG archive"
	}
	return "plain unencrypted tarball"
}

type restorePlan struct {
	archive   string
	kind      restoreArchiveKind
	manifest  string // "" when there is none
	codeRoot  string
	stateRoot string
}

const restoreUsage = "usage: plugroot restore [check] [directory-or-archive]"

func cmdRestore(root string, args []string) (int, error) {
	first := ""
	if len(args) > 0 {
		first = args[0]
	}
	switch first {
	case "check":
		source, err := restoreSourceArg(args[1:])
		if err != nil {
			return 0, err
		}
		plan, err := restorePlanForSource(root, source)
		if err != nil {
			return 0, err
		}
		return 0, printRestorePlan(plan, true)
	case "finish":
		return cmdRestoreFinish(root, args[1:])
	case "-h", "--help":
		printRestoreHelp()
		return 0, nil
	}
	source, err := restoreSourceArg(args)
	if err != nil {
		return 0, err
	}
	plan, err := restorePlanForSource(root, source)
	if err != nil {
		return 0, err
	}
	if err := printRestorePlan(plan, false); err != nil {
		return 0, err
	}
	return runRestorePlan(plan)
}

func restoreSourceArg(args []string) (string, error) {
	if len(args) > 1 {
		return "", errors.New(restoreUsage)
	}
	if len(args) == 1 {
		if strings.HasPrefix(args[0], "-") {
			return "", errors.New("restore uses words, not flags: try `plugroot restore check /path/to/usb`")
		}
		return args[0], nil
	}
	return defaultRestoreSourceDir(), nil
}

func printRestoreHelp() {
	fmt.Fprintln(os.Stderr, "usage: plugroot restore check [directory-or-archive]\n       plugroot restore [directory-or-archive]\n       plugroot restore finish")
}

// confirmPhrase asks on stderr and reads one line from stdin.
func confirmPhrase(expected, prompt string) (bool, error) {
	fmt.Fprintln(os.Stderr, prompt)
	fmt.Fprint(os.Stderr, "> ")
	line, err := bufio.NewReader(os.Stdin).ReadString('\n')
	if err != nil && line == "" && !errors.Is(err, io.EOF) {
		return false, ioError(err)
	}
	return strings.TrimSpace(line) == expected, nil
}

func defaultRestoreSourceDir() string {
	if home, ok := currentOperatorHome(); ok {
		return joinPath(home, "Documents/plugroot-emergency-exports")
	}
	if home, ok := os.LookupEnv("HOME"); ok {
		return joinPath(home, "plugroot-emergency-exports")
	}
	return "/tmp/plugroot-emergency-exports"
}

func currentOperatorHome() (string, bool) {
	home, ok := "", false
	if user := currentOperatorUser(); user != nil {
		home, ok = userHome(*user)
	}
	if !ok {
		home, ok = os.LookupEnv("HOME")
	}
	if !ok || strings.TrimSpace(home) == "" {
		return "", false
	}
	return home, true
}

func currentOperatorUser() *string {
	if user := usableUser(os.LookupEnv("SUDO_USER")); user != nil {
		return user
	}
	if !isRoot() {
		if user := usableUser(commandText("id", "-un")); user != nil {
			return user
		}
	}
	return usableUser(os.LookupEnv("USER"))
}

func restorePlanForSource(root, source string) (*restorePlan, error) {
	source, err := absoluteCleanPath(source)
	if err != nil {
		return nil, err
	}
	archive, err := findRestoreArchive(source)
	if err != nil {
		return nil, err
	}
	kind, ok := classifyRestoreArchive(archive)
	if !ok {
		return nil, fmt.Errorf("not a Plugroot backup archive: %s", archive)
	}
	manifest := companionRestoreManifest(archive, kind)
	codeRoot, stateRoot, err := restoreTargetsFromManifest(root, manifest)
	if err != nil {
		return nil, err
	}
	plan := &restorePlan{archive: archive, kind: kind, manifest: manifest, codeRoot: codeRoot, stateRoot: stateRoot}
	if err := validateRestoreTargets(plan); err != nil {
		return nil, err
	}
	return plan, nil
}

func findRestoreArchive(source string) (string, error) {
	if isFile(source) {
		if _, ok := classifyRestoreArchive(source); ok {
			return source, nil
		}
		return "", fmt.Errorf("not a supported Plugroot backup archive: %s", source)
	}
	if !exists(source) {
		return "", fmt.Errorf("restore source does not exist: %s", source)
	}
	if !isDir(source) {
		return "", fmt.Errorf("restore source is not a directory or archive: %s", source)
	}
	f, err := os.Open(source)
	if err != nil {
		return "", ioError(err)
	}
	entries, err := f.ReadDir(-1)
	f.Close()
	if err != nil {
		return "", ioError(err)
	}
	type candidate struct {
		priority int
		modified time.Time
		path     string
	}
	var candidates []candidate
	for _, entry := range entries {
		path := joinPath(source, entry.Name())
		kind, ok := classifyRestoreArchive(path)
		if !ok {
			continue
		}
		modified := time.Unix(0, 0)
		if info, err := os.Lstat(path); err == nil {
			modified = info.ModTime()
		}
		priority := 0
		if kind == restoreEncrypted {
			priority = 1
		}
		candidates = append(candidates, candidate{priority, modified, path})
	}
	sort.SliceStable(candidates, func(a, b int) bool {
		if candidates[a].priority != candidates[b].priority {
			return candidates[a].priority > candidates[b].priority
		}
		return candidates[a].modified.After(candidates[b].modified)
	})
	if len(candidates) == 0 {
		return "", fmt.Errorf("no Plugroot backup archive found in %s", source)
	}
	return candidates[0].path, nil
}

func classifyRestoreArchive(path string) (restoreArchiveKind, bool) {
	name, ok := fileName(path)
	if !ok {
		return 0, false
	}
	switch {
	case strings.HasSuffix(name, ".tar.zst.gpg"):
		return restoreEncrypted, true
	case strings.HasSuffix(name, ".tar.zst"):
		return restorePlain, true
	}
	return 0, false
}

func companionRestoreManifest(archive string, kind restoreArchiveKind) string {
	name, ok := fileName(archive)
	if !ok {
		return ""
	}
	suffix := ".tar.zst"
	if kind == restoreEncrypted {
		suffix = ".tar.zst.gpg"
	}
	stem, found := strings.CutSuffix(name, suffix)
	if !found {
		return ""
	}
	dir, _ := parentPath(archive)
	for _, candidate := range []string{joinPath(dir, stem+".manifest.json"), joinPath(dir, stem+".manifest.txt")} {
		if exists(candidate) {
			return candidate
		}
	}
	return ""
}

func restoreTargetsFromManifest(root, manifest string) (string, string, error) {
	codeRoot, err := absoluteCleanPath(root)
	if err != nil {
		return "", "", err
	}
	if manifest == "" {
		return codeRoot, defaultStateRoot(codeRoot), nil
	}
	name, _ := fileName(manifest)
	if ext, ok := extension(name); !ok || ext != "json" {
		return codeRoot, defaultStateRoot(codeRoot), nil
	}
	raw, err := readText(manifest)
	if err != nil {
		return "", "", err
	}
	parsed, err := decodeEmergencyManifest(raw)
	if err != nil {
		return "", "", err
	}
	return cleanPath(parsed.CodeRoot), cleanPath(parsed.StateRoot), nil
}

func printRestorePlan(plan *restorePlan, checkOnly bool) error {
	fmt.Printf("restore %s\n", pick(checkOnly, "check", "plan"))
	fmt.Printf("backup: %s\n", plan.archive)
	fmt.Printf("type: %s\n", plan.kind.label())
	if plan.manifest != "" {
		fmt.Printf("manifest: %s\n", plan.manifest)
	} else {
		fmt.Println("manifest: missing; using default roots")
	}
	fmt.Printf("target code root: %s\n", plan.codeRoot)
	fmt.Printf("target state root: %s\n", plan.stateRoot)
	fmt.Printf("compatibility link: /var/lib/plugroot -> %s\n", plan.stateRoot)
	if plan.kind == restorePlain {
		fmt.Println("warning: this archive is not encrypted")
	}
	if err := validateRestoreTargets(plan); err != nil {
		return err
	}
	for _, path := range []string{plan.codeRoot, plan.stateRoot, "/var/lib/plugroot"} {
		fmt.Printf("target %s: %s\n", path, pick(exists(path), "exists", "clear"))
	}
	tools := []string{"tar", "zstd", "systemctl"}
	if plan.kind == restoreEncrypted {
		tools = append(tools, "gpg")
	}
	tools = append(tools, "git", "docker")
	fmt.Println("tools:")
	for _, tool := range tools {
		fmt.Printf("  - %s: %s\n", tool, pick(programAvailable(tool), "found", "missing"))
	}
	compose := runCommand("docker", []string{"compose", "version"}, "", nil)
	fmt.Printf("  - docker compose: %s\n", pick(compose.Code == 0, "found", "missing"))
	fmt.Println("flow:")
	fmt.Println("  1. install host prerequisites for your distro")
	fmt.Printf("  2. plugroot restore %s\n", plan.archive)
	fmt.Println("  3. plugroot restore finish")
	fmt.Println("  4. plugroot doctor")
	return nil
}

func validateRestoreTargets(plan *restorePlan) error {
	codeRoot, stateRoot := cleanPath(plan.codeRoot), cleanPath(plan.stateRoot)
	if !isAbs(codeRoot) || !isAbs(stateRoot) {
		return errors.New("restore targets must be absolute paths")
	}
	if codeRoot == "/" || stateRoot == "/" {
		return errors.New("restore refuses to use the filesystem root as a target")
	}
	if sameOrInside(codeRoot, stateRoot) || sameOrInside(stateRoot, codeRoot) {
		return errors.New("code and state restore targets must be separate directories")
	}
	return nil
}

func restoreArchiveRoots(plan *restorePlan) ([2]string, error) {
	codeRoot, ok := tarRelativePath(plan.codeRoot)
	if !ok {
		return [2]string{}, errors.New("code restore target cannot be the filesystem root")
	}
	stateRoot, ok := tarRelativePath(plan.stateRoot)
	if !ok {
		return [2]string{}, errors.New("state restore target cannot be the filesystem root")
	}
	return [2]string{codeRoot, stateRoot}, nil
}

func runRestorePlan(plan *restorePlan) (int, error) {
	if !isRoot() {
		return 0, errors.New("restore requires root; the installed plugroot launcher will use sudo automatically")
	}
	if err := validateRestoreTargets(plan); err != nil {
		return 0, err
	}
	for _, program := range []string{"tar", "zstd"} {
		if !programAvailable(program) {
			return 0, fmt.Errorf("required program not found in PATH: %s", program)
		}
	}
	if plan.kind == restoreEncrypted && !programAvailable("gpg") {
		return 0, errors.New("required program not found in PATH: gpg")
	}
	for _, path := range []string{plan.codeRoot, plan.stateRoot, "/var/lib/plugroot"} {
		if exists(path) {
			return 0, fmt.Errorf("restore refuses to overwrite existing path: %s", path)
		}
	}
	confirmed, err := confirmPhrase("RESTORE", "Type RESTORE to extract Plugroot roots onto this host")
	if err != nil {
		return 0, err
	}
	if !confirmed {
		fmt.Fprintln(os.Stderr, "restore cancelled")
		return 2, nil
	}
	tarPath, tempTar := plan.archive, ""
	if plan.kind == restoreEncrypted {
		tempTar = joinPath(tempDir(), fmt.Sprintf("plugroot-restore-%s.tar.zst", emergencyTimestamp()))
		out := runCommand("gpg", []string{"--decrypt", "--output", tempTar, plan.archive}, "", nil)
		if out.Code != 0 {
			os.Remove(tempTar)
			return 0, fmt.Errorf("gpg decrypt failed: %s", out.Text)
		}
		tarPath = tempTar
	}
	roots, err := restoreArchiveRoots(plan)
	if err != nil {
		return 0, err
	}
	for _, root := range roots {
		out := runCommand("tar", []string{"--zstd", "-tf", tarPath, root}, "", nil)
		if out.Code != 0 {
			return 0, fmt.Errorf("archive is missing %s: %s", root, out.Text)
		}
	}
	out := runCommand("tar", []string{"--zstd", "-xpf", tarPath, "-C", "/", roots[0], roots[1]}, "", nil)
	if out.Code != 0 {
		return 0, fmt.Errorf("tar extract failed: %s", out.Text)
	}
	if tempTar != "" {
		os.Remove(tempTar)
	}
	if !exists("/var/lib/plugroot") {
		if err := os.Symlink(plan.stateRoot, "/var/lib/plugroot"); err != nil {
			return 0, ioError(err)
		}
	}
	fmt.Printf("restored %s and %s\n", plan.codeRoot, plan.stateRoot)
	fmt.Println("next: install any missing host prerequisites, then run `plugroot restore finish`")
	return 0, nil
}

func cmdRestoreFinish(root string, args []string) (int, error) {
	if len(args) > 0 {
		printRestoreHelp()
		return 2, nil
	}
	if !isRoot() {
		return 0, errors.New("restore finish requires root; the installed plugroot launcher will use sudo automatically")
	}
	root, err := absoluteCleanPath(root)
	if err != nil {
		return 0, err
	}
	bin := joinPath(root, "bin/plugroot")
	if !exists(bin) {
		return 0, fmt.Errorf("plugroot binary not found: %s", bin)
	}
	fmt.Println("checking restored Plugroot roots")
	if err := ensureSuccess(runCommand(bin, []string{"--root", root, "boundary", "--strict"}, "", nil)); err != nil {
		return 0, err
	}
	fmt.Println("previewing apply")
	printCmdOutput(runCommand(bin, []string{"--root", root, "apply", "--dry-run"}, "", nil))
	confirmed, err := confirmPhrase("APPLY", "Type APPLY to install units and start services")
	if err != nil {
		return 0, err
	}
	if !confirmed {
		fmt.Fprintln(os.Stderr, "restore finish cancelled")
		return 2, nil
	}
	if err := ensureSuccess(runCommand(bin, []string{"--root", root, "apply"}, "", nil)); err != nil {
		return 0, err
	}
	if err := ensureSuccess(runCommand(bin, []string{"--root", root, "up", "all"}, "", nil)); err != nil {
		return 0, err
	}
	doctor := runCommand(bin, []string{"--root", root, "doctor"}, "", nil)
	printCmdOutput(doctor)
	return min(doctor.Code, 255), nil
}
