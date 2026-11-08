package main

// backup and emergency export: an encrypted rescue bundle of the code root,
// the state root and the host's edge configuration.

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

type emergencyExportOptions struct {
	outputDir       string
	yes             bool
	dryRun          bool
	passphraseFile  *string
	createOutputDir bool
	ownerUser       *string
}

type emergencySource struct {
	label string
	path  string
}

type emergencyCommandSpec struct {
	fileName string
	program  string
	args     []string
}

// emergencyExportManifest is the companion .manifest.json, also read back
// by restore.
type emergencyExportManifest struct {
	Version          uint8    `json:"version"`
	CreatedUTC       string   `json:"created_utc"`
	CodeRoot         string   `json:"code_root"`
	StateRoot        string   `json:"state_root"`
	OutputDir        string   `json:"output_dir"`
	EncryptedArchive string   `json:"encrypted_archive"`
	IncludedPaths    []string `json:"included_paths"`
	SkippedPaths     []string `json:"skipped_paths"`
	MetadataCommands []string `json:"metadata_commands"`
}

func (m emergencyExportManifest) json() jsonObject {
	return jsonObject{
		{"version", int(m.Version)}, {"created_utc", m.CreatedUTC}, {"code_root", m.CodeRoot},
		{"state_root", m.StateRoot}, {"output_dir", m.OutputDir}, {"encrypted_archive", m.EncryptedArchive},
		{"included_paths", m.IncludedPaths}, {"skipped_paths", m.SkippedPaths},
		{"metadata_commands", m.MetadataCommands},
	}
}

const emergencyUsage = "usage: plugroot emergency export --output <directory> --yes [--dry-run] [--passphrase-file <path>]"

func cmdBackup(ctx *Context, args []string) (int, error) {
	outputDir, dryRun, ok, err := parseSimpleBackupArgs(args, defaultEmergencyOutputDir(ctx))
	if err != nil {
		return 0, err
	}
	if !ok {
		return 2, nil
	}
	if !dryRun {
		fmt.Println("encrypted backup: GPG will ask for a passphrase")
		fmt.Println("keep that passphrase outside this laptop and outside Vaultwarden")
	}
	return runEmergencyExport(ctx, emergencyExportOptions{
		outputDir: outputDir, yes: true, dryRun: dryRun, createOutputDir: true,
		ownerUser: emergencyOutputOwnerUser(ctx),
	})
}

func cmdEmergency(ctx *Context, args []string) (int, error) {
	first := ""
	if len(args) > 0 {
		first = args[0]
	}
	switch first {
	case "save":
		return cmdBackup(ctx, args[1:])
	case "check":
		return cmdBackup(ctx, append([]string{"check"}, args[1:]...))
	case "export":
		options, ok, err := parseEmergencyExportOptions(args[1:])
		if err != nil {
			return 0, err
		}
		if !ok {
			return 2, nil
		}
		return runEmergencyExport(ctx, options)
	}
	fmt.Fprintln(os.Stderr, "usage: plugroot backup [check] [directory]\n       plugroot emergency save [directory]\n       "+strings.TrimPrefix(emergencyUsage, "usage: "))
	return 2, nil
}

func runEmergencyExport(ctx *Context, options emergencyExportOptions) (int, error) {
	if !options.yes && !options.dryRun {
		fmt.Fprintln(os.Stderr, "refusing to export without --yes; run --dry-run first to inspect the plan")
		return 2, nil
	}
	outputDir, err := absoluteCleanPath(options.outputDir)
	if err != nil {
		return 0, err
	}
	if err := validateEmergencyOutputBoundary(ctx, outputDir); err != nil {
		return 0, err
	}
	if options.createOutputDir {
		if err := os.MkdirAll(outputDir, 0o777); err != nil {
			return 0, ioError(err)
		}
		if err := chownPathToUser(outputDir, options.ownerUser); err != nil {
			return 0, err
		}
	}
	if err := validateEmergencyOutputDir(outputDir); err != nil {
		return 0, err
	}
	timestamp := emergencyTimestamp()
	stem := fmt.Sprintf("plugroot-emergency-%s-%s", sanitizeFilenameComponent(emergencyHostName(ctx)), timestamp)
	tarPath := joinPath(outputDir, stem+".tar.zst")
	encryptedPath := joinPath(outputDir, stem+".tar.zst.gpg")
	companionPath := joinPath(outputDir, stem+".manifest.json")
	metadataDir := joinPath(tempDir(), stem+"-metadata")

	included, skipped := emergencySources(ctx)
	manifest := emergencyExportManifest{
		Version: 1, CreatedUTC: timestamp, CodeRoot: cleanPath(ctx.Root), StateRoot: cleanPath(ctx.stateRoot()),
		OutputDir: outputDir, EncryptedArchive: encryptedPath,
		IncludedPaths: []string{}, SkippedPaths: []string{}, MetadataCommands: []string{},
	}
	for _, s := range included {
		manifest.IncludedPaths = append(manifest.IncludedPaths, s.path)
	}
	for _, s := range skipped {
		manifest.SkippedPaths = append(manifest.SkippedPaths, fmt.Sprintf("%s (%s)", s.path, s.label))
	}
	for _, spec := range emergencyMetadataCommands() {
		manifest.MetadataCommands = append(manifest.MetadataCommands, commandLineForSpec(spec))
	}
	printEmergencyExportPlan(manifest, encryptedPath, options.dryRun, options.passphraseFile)
	if options.dryRun {
		return 0, nil
	}
	if err := validateEmergencyTools(options.passphraseFile); err != nil {
		return 0, err
	}
	if exists(tarPath) || exists(encryptedPath) || exists(companionPath) {
		return 0, errors.New("refusing to overwrite an existing emergency export file")
	}
	if err := os.Mkdir(metadataDir, 0o777); err != nil {
		return 0, ioError(err)
	}
	exportErr := func() error {
		if err := writeEmergencyMetadata(metadataDir, manifest); err != nil {
			return err
		}
		if err := writeJSONFile(companionPath, manifest.json()); err != nil {
			return err
		}
		if err := createEmergencyTar(ctx, included, metadataDir, tarPath); err != nil {
			return err
		}
		if err := encryptEmergencyTar(tarPath, encryptedPath, options.passphraseFile); err != nil {
			return err
		}
		if err := chownPathToUser(encryptedPath, options.ownerUser); err != nil {
			return err
		}
		if err := chownPathToUser(companionPath, options.ownerUser); err != nil {
			return err
		}
		fmt.Printf("wrote encrypted export: %s\n", encryptedPath)
		fmt.Printf("wrote companion manifest: %s\n", companionPath)
		return nil
	}()
	os.Remove(tarPath)
	os.RemoveAll(metadataDir)
	if exportErr != nil {
		return 0, exportErr
	}
	return 0, nil
}

// tempDir is Rust's env::temp_dir: $TMPDIR, else /tmp.
func tempDir() string {
	if dir, ok := os.LookupEnv("TMPDIR"); ok {
		return dir
	}
	return "/tmp"
}

// parseSimpleBackupArgs reads `backup [check] [directory]`; ok is false
// after printing usage for -h.
func parseSimpleBackupArgs(args []string, defaultOutput string) (string, bool, bool, error) {
	dryRun := false
	rest := args
	if len(rest) > 0 && (rest[0] == "check" || rest[0] == "preview" || rest[0] == "dry-run") {
		dryRun = true
		rest = rest[1:]
	}
	for _, arg := range rest {
		if arg == "-h" || arg == "--help" {
			fmt.Fprintln(os.Stderr, "usage: plugroot backup [check] [directory]")
			return "", false, false, nil
		}
	}
	if len(rest) > 1 {
		return "", false, false, errors.New("usage: plugroot backup [check] [directory]")
	}
	if len(rest) == 1 && strings.HasPrefix(rest[0], "-") {
		return "", false, false, errors.New("backup uses words, not flags: try `plugroot backup check` or `plugroot backup /path/to/usb`")
	}
	output := defaultOutput
	if len(rest) == 1 {
		output = rest[0]
	}
	return output, dryRun, true, nil
}

func parseEmergencyExportOptions(args []string) (emergencyExportOptions, bool, error) {
	var options emergencyExportOptions
	outputGiven := false
	for i := 0; i < len(args); i++ {
		switch arg := args[i]; {
		case arg == "--output":
			if i+1 >= len(args) {
				return options, false, errors.New("--output requires a directory")
			}
			options.outputDir, outputGiven = args[i+1], true
			i++
		case strings.HasPrefix(arg, "--output="):
			options.outputDir, outputGiven = trimAllPrefix(arg, "--output="), true
		case arg == "--yes":
			options.yes = true
		case arg == "--dry-run":
			options.dryRun = true
		case arg == "--passphrase-file":
			if i+1 >= len(args) {
				return options, false, errors.New("--passphrase-file requires a path")
			}
			path := args[i+1]
			options.passphraseFile = &path
			i++
		case strings.HasPrefix(arg, "--passphrase-file="):
			path := trimAllPrefix(arg, "--passphrase-file=")
			options.passphraseFile = &path
		case arg == "-h" || arg == "--help":
			fmt.Fprintln(os.Stderr, emergencyUsage)
			return options, false, nil
		default:
			return options, false, fmt.Errorf("unknown emergency export option: %s", arg)
		}
	}
	if !outputGiven {
		fmt.Fprintln(os.Stderr, emergencyUsage)
		return options, false, nil
	}
	return options, true, nil
}

func validateEmergencyOutputBoundary(ctx *Context, outputDir string) error {
	if sameOrInside(cleanPath(ctx.Root), outputDir) {
		return errors.New("output directory must not be inside the Plugroot code checkout")
	}
	if sameOrInside(cleanPath(ctx.stateRoot()), outputDir) {
		return errors.New("output directory must not be inside the Plugroot state root")
	}
	return nil
}

func validateEmergencyOutputDir(outputDir string) error {
	if !exists(outputDir) {
		return fmt.Errorf("output directory does not exist: %s", outputDir)
	}
	if !isDir(outputDir) {
		return fmt.Errorf("output path is not a directory: %s", outputDir)
	}
	return nil
}

func validateEmergencyTools(passphraseFile *string) error {
	for _, program := range []string{"tar", "gpg", "zstd"} {
		if !programAvailable(program) {
			return fmt.Errorf("required program not found in PATH: %s", program)
		}
	}
	if passphraseFile != nil && !isFile(*passphraseFile) {
		return fmt.Errorf("passphrase file not found: %s", *passphraseFile)
	}
	return nil
}

func defaultEmergencyOutputDir(ctx *Context) string {
	if home, ok := emergencyOperatorHome(ctx); ok {
		return joinPath(home, "Documents/plugroot-emergency-exports")
	}
	if home, ok := os.LookupEnv("HOME"); ok {
		return joinPath(home, "plugroot-emergency-exports")
	}
	return "/tmp/plugroot-emergency-exports"
}

func emergencyOperatorHome(ctx *Context) (string, bool) {
	home, ok := "", false
	if user := emergencyOutputOwnerUser(ctx); user != nil {
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

// usableUser is a user name that is set, not blank and not root.
func usableUser(user string, ok bool) *string {
	if !ok || user == "root" || strings.TrimSpace(user) == "" {
		return nil
	}
	return &user
}

func emergencyOutputOwnerUser(ctx *Context) *string {
	if user := usableUser(os.LookupEnv("SUDO_USER")); user != nil {
		return user
	}
	if !isRoot() {
		if user := usableUser(commandText("id", "-un")); user != nil {
			return user
		}
	}
	if host := ctx.Manifest.Host; host != nil && host.User != nil {
		if user := usableUser(*host.User, true); user != nil {
			return user
		}
	}
	return usableUser(os.LookupEnv("USER"))
}

func chownPathToUser(path string, user *string) error {
	if !isRoot() || user == nil || strings.TrimSpace(*user) == "" {
		return nil
	}
	group, ok := commandText("id", "-gn", *user)
	if !ok {
		group = *user
	}
	return ensureSuccess(runCommand("chown", []string{*user + ":" + group, path}, "", nil))
}

const emergencyReadme = "Plugroot emergency export metadata. The companion .manifest.json next to the encrypted archive is intentionally non-secret; this directory is inside the encrypted archive and may contain private host inventory.\n"

func writeEmergencyMetadata(dir string, manifest emergencyExportManifest) error {
	if err := writeJSONFile(joinPath(dir, "manifest.json"), manifest.json()); err != nil {
		return err
	}
	if err := os.WriteFile(joinPath(dir, "README.txt"), []byte(emergencyReadme), 0o666); err != nil {
		return ioError(err)
	}
	for _, spec := range emergencyMetadataCommands() {
		out := runCommand(spec.program, spec.args, "", nil)
		text := fmt.Sprintf("$ %s\nexit: %d\n\n%s\n", commandLineForSpec(spec), out.Code, out.Text)
		if err := os.WriteFile(joinPath(dir, spec.fileName), []byte(text), 0o666); err != nil {
			return ioError(err)
		}
	}
	return nil
}

func writeJSONFile(path string, value jsonObject) error {
	return ioError(os.WriteFile(path, []byte(prettyJSON(value)+"\n"), 0o666))
}

func createEmergencyTar(ctx *Context, sources []emergencySource, metadataDir, tarPath string) error {
	args := []string{"--zstd", "-cf", tarPath}
	for _, exclude := range emergencyTarExcludes(ctx) {
		args = append(args, "--exclude="+exclude)
	}
	args = append(args, "-C", "/")
	for _, s := range sources {
		if path, ok := tarRelativePath(s.path); ok {
			args = append(args, path)
		}
	}
	parent, ok := parentPath(metadataDir)
	if !ok {
		parent = "/"
	}
	name, ok := fileName(metadataDir)
	if !ok {
		return errors.New("invalid metadata path")
	}
	args = append(args, "-C", parent, name)
	out := runCommand("tar", args, "", nil)
	if out.Code == 0 {
		return nil
	}
	return fmt.Errorf("tar failed: %s", out.Text)
}

func encryptEmergencyTar(tarPath, encryptedPath string, passphraseFile *string) error {
	var args []string
	if passphraseFile != nil {
		args = append(args, "--batch", "--yes", "--pinentry-mode", "loopback", "--passphrase-file", *passphraseFile)
	}
	args = append(args, "--symmetric", "--cipher-algo", "AES256", "--output", encryptedPath, tarPath)
	out := runCommand("gpg", args, "", nil)
	if out.Code == 0 {
		return nil
	}
	os.Remove(encryptedPath)
	return fmt.Errorf("gpg failed: %s", out.Text)
}

func printEmergencyExportPlan(m emergencyExportManifest, encryptedPath string, dryRun bool, passphraseFile *string) {
	fmt.Printf("emergency export %s\n", pick(dryRun, "dry run", "plan"))
	fmt.Printf("code root: %s\n", m.CodeRoot)
	fmt.Printf("state root: %s\n", m.StateRoot)
	fmt.Printf("output: %s\n", encryptedPath)
	mode := "interactive prompt"
	if passphraseFile != nil {
		mode = "passphrase file"
	}
	fmt.Printf("encryption: gpg symmetric AES256 (%s)\n", mode)
	fmt.Println("included paths:")
	for _, path := range m.IncludedPaths {
		fmt.Printf("  - %s\n", path)
	}
	if len(m.SkippedPaths) > 0 {
		fmt.Println("skipped missing paths:")
		for _, path := range m.SkippedPaths {
			fmt.Printf("  - %s\n", path)
		}
	}
}

func emergencyTarExcludes(ctx *Context) []string {
	codeRoot, stateRoot := cleanPath(ctx.Root), cleanPath(ctx.stateRoot())
	var excludes []string
	for _, name := range []string{"target", ".cache", "node_modules"} {
		if relative, ok := tarRelativePath(joinPath(codeRoot, name)); ok {
			excludes = append(excludes, relative)
		}
	}
	if relative, ok := tarRelativePath(stateRoot); ok {
		for _, pattern := range []string{"repos/*/target", "repos/*/.cache", "repos/*/node_modules"} {
			excludes = append(excludes, relative+"/"+pattern)
		}
	}
	return excludes
}

func emergencyTimestamp() string {
	if stamp, ok := commandText("date", "-u", "+%Y%m%dT%H%M%SZ"); ok {
		return stamp
	}
	return strconv.FormatInt(time.Now().Unix(), 10)
}

func emergencyHostName(ctx *Context) string {
	if host := ctx.Manifest.Host; host != nil && host.Name != nil && strings.TrimSpace(*host.Name) != "" {
		return *host.Name
	}
	if name, ok := commandText("hostname"); ok {
		return name
	}
	return "plugroot-host"
}

func sanitizeFilenameComponent(value string) string {
	var b strings.Builder
	for _, r := range value {
		if r < 0x80 && (r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '.' || r == '_' || r == '-') {
			b.WriteRune(r)
		} else {
			b.WriteByte('-')
		}
	}
	if sanitized := strings.Trim(b.String(), "-"); sanitized != "" {
		return sanitized
	}
	return "plugroot-host"
}

func commandLineForSpec(spec emergencyCommandSpec) string {
	return strings.Join(append([]string{spec.program}, spec.args...), " ")
}

// programAvailable is whether `program --version` can be started at all.
func programAvailable(program string) bool {
	cmd := exec.Command(program, "--version")
	return cmd.Run() == nil || cmd.ProcessState != nil
}

func emergencySources(ctx *Context) ([]emergencySource, []emergencySource) {
	candidates := []emergencySource{
		{"code root", cleanPath(ctx.Root)},
		{"state root", cleanPath(ctx.stateRoot())},
		{"rustdesk system config", "/etc/rustdesk"},
		{"ufw config", "/etc/ufw"},
		{"tailscale config", "/etc/tailscale"},
		{"docker config", "/etc/docker"},
	}
	for _, path := range []string{"/etc/hostname", "/etc/hosts", "/etc/fstab", "/etc/os-release", "/etc/systemd/resolved.conf"} {
		candidates = append(candidates, emergencySource{"system edge config", path})
	}
	for _, path := range emergencySystemdUnitPaths(ctx) {
		candidates = append(candidates, emergencySource{"systemd unit", path})
	}
	if path, ok := emergencyUserRustdeskConfig(ctx); ok {
		candidates = append(candidates, emergencySource{"rustdesk user config", path})
	}
	seen := map[string]bool{}
	var included, skipped []emergencySource
	for _, s := range candidates {
		key := cleanPath(s.path)
		if seen[key] {
			continue
		}
		seen[key] = true
		if exists(s.path) {
			included = append(included, s)
		} else {
			skipped = append(skipped, s)
		}
	}
	return included, skipped
}

func emergencySystemdUnitPaths(ctx *Context) []string {
	var paths []string
	for _, s := range ctx.Manifest.Service {
		if s.Unit == nil {
			continue
		}
		if s.Kind == "user-systemd" {
			if s.User != nil {
				if home, ok := userHome(*s.User); ok {
					paths = append(paths, joinPath(joinPath(home, ".config/systemd/user"), *s.Unit))
				}
			}
			continue
		}
		paths = append(paths, joinPath("/etc/systemd/system", *s.Unit))
	}
	if f, err := os.Open("/etc/systemd/system"); err == nil {
		entries, _ := f.ReadDir(-1)
		f.Close()
		for _, entry := range entries {
			name := entry.Name()
			if strings.HasPrefix(name, "plugroot-") && (strings.HasSuffix(name, ".service") || strings.HasSuffix(name, ".timer")) {
				paths = append(paths, joinPath("/etc/systemd/system", name))
			}
		}
	}
	return paths
}

func emergencyUserRustdeskConfig(ctx *Context) (string, bool) {
	var user string
	switch {
	case ctx.Manifest.Host != nil && ctx.Manifest.Host.User != nil:
		user = *ctx.Manifest.Host.User
	default:
		if u, ok := os.LookupEnv("SUDO_USER"); ok && u != "root" {
			user = u
		} else if u, ok := os.LookupEnv("USER"); ok && u != "root" {
			user = u
		} else {
			return "", false
		}
	}
	home, ok := userHome(user)
	if !ok {
		return "", false
	}
	return joinPath(home, ".config/rustdesk"), true
}

func emergencyMetadataCommands() []emergencyCommandSpec {
	return []emergencyCommandSpec{
		{"uname.txt", "uname", []string{"-a"}},
		{"df.txt", "df", []string{"-hT"}},
		{"lsblk.txt", "lsblk", []string{"-f"}},
		{"systemd-failed.txt", "systemctl", []string{"list-units", "--failed", "--no-pager"}},
		{"systemd-enabled.txt", "systemctl", []string{"list-unit-files", "--state=enabled", "--no-pager"}},
		{"docker-ps.txt", "docker", []string{"ps", "-a"}},
		{"docker-compose-ls.txt", "docker", []string{"compose", "ls"}},
		{"docker-volumes.txt", "docker", []string{"volume", "ls"}},
		{"tailscale-status.txt", "tailscale", []string{"status"}},
		{"tailscale-serve.txt", "tailscale", []string{"serve", "status"}},
		{"ufw-status.txt", "ufw", []string{"status", "verbose"}},
		{"dpkg-packages.txt", "dpkg-query", []string{"-W", "-f=${binary:Package}\t${Version}\n"}},
		{"flatpak-apps.txt", "flatpak", []string{"list", "--app", "--columns=application,version,branch,origin"}},
	}
}

// decodeEmergencyManifest reads a companion manifest: every field present,
// of its type.
func decodeEmergencyManifest(raw string) (emergencyExportManifest, error) {
	var m emergencyExportManifest
	var fields map[string]json.RawMessage
	if err := json.Unmarshal([]byte(raw), &fields); err != nil {
		return m, err
	}
	for _, name := range []string{"version", "created_utc", "code_root", "state_root", "output_dir", "encrypted_archive", "included_paths", "skipped_paths", "metadata_commands"} {
		value, ok := fields[name]
		if !ok {
			return m, fmt.Errorf("missing field `%s`", name)
		}
		if string(value) == "null" {
			return m, fmt.Errorf("invalid type: null for field `%s`", name)
		}
	}
	if err := json.Unmarshal([]byte(raw), &m); err != nil {
		return m, err
	}
	return m, nil
}
