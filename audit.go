package main

// audit-public: tracked files checked for private paths, secret markers,
// private-network addresses and the host's own denylist terms.

import (
	"bytes"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"sort"
	"strconv"
	"strings"
)

type auditFinding struct {
	path    string
	line    int // 0 when the finding is about the path
	message string
}

func (f auditFinding) String() string {
	if f.line > 0 {
		return fmt.Sprintf("%s:%d: %s", f.path, f.line, f.message)
	}
	return fmt.Sprintf("%s: %s", f.path, f.message)
}

const auditHelp = `Usage:
  plugroot audit-public
  plugroot audit-public --install-hook

Checks tracked files for private paths, common secret markers, private-network
IP leaks, and host-specific denylist terms.
`

func cmdAuditPublic(root string, args []string) (int, error) {
	for _, arg := range args {
		if arg == "-h" || arg == "--help" {
			fmt.Print(auditHelp + "\n")
			return 0, nil
		}
	}
	for _, arg := range args {
		if arg == "--install-hook" {
			if err := installAuditHook(root); err != nil {
				return 0, err
			}
			fmt.Println("installed .git/hooks/pre-commit and .git/hooks/pre-push")
			break
		}
	}
	findings, err := auditPublic(root)
	if err != nil {
		return 0, err
	}
	if len(findings) == 0 {
		fmt.Println("audit-public: ok")
		return 0, nil
	}
	fmt.Fprintf(os.Stderr, "audit-public: found %d issue(s)\n", len(findings))
	for _, f := range findings {
		fmt.Fprintln(os.Stderr, f.String())
	}
	return 1, nil
}

// auditHook runs the audit and the boundary check before a commit or push.
const auditHook = `#!/bin/sh
set -eu
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"
go run . audit-public
test -f plugroot.toml && go run . boundary --strict
`

func installAuditHook(root string) error {
	hooks := joinPath(root, ".git/hooks")
	if err := os.MkdirAll(hooks, 0o777); err != nil {
		return ioError(err)
	}
	for _, name := range []string{"pre-commit", "pre-push"} {
		hook := joinPath(hooks, name)
		if err := os.WriteFile(hook, []byte(auditHook), 0o666); err != nil {
			return ioError(err)
		}
		if err := ensureSuccess(runCommand("chmod", []string{"0755", hook}, "", nil)); err != nil {
			return err
		}
	}
	return nil
}

func auditPublic(root string) ([]auditFinding, error) {
	files, err := gitTrackedFiles(root)
	if err != nil {
		return nil, err
	}
	terms := loadAuditDenylist(root)
	var findings []auditFinding
	for _, path := range files {
		if message, ok := auditPath(path); ok {
			findings = append(findings, auditFinding{path: path, message: message})
			continue
		}
		full := joinPath(root, path)
		if info, err := os.Stat(full); err == nil && info.Size() > 1_000_000 {
			continue
		}
		text, err := readText(full)
		if err != nil {
			continue
		}
		for i, line := range rustLines(text) {
			for _, message := range auditLine(line, terms) {
				findings = append(findings, auditFinding{path: path, line: i + 1, message: message})
			}
		}
	}
	return findings, nil
}

func gitTrackedFiles(root string) ([]string, error) {
	cmd := exec.Command("git", "ls-files", "-z")
	cmd.Dir = root
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	if err := cmd.Run(); err != nil {
		var exit *exec.ExitError
		if !errors.As(err, &exit) {
			return nil, errors.New(startError(err))
		}
		return nil, errors.New(strings.TrimSpace(lossyUTF8(stderr.Bytes())))
	}
	var files []string
	for _, path := range strings.Split(lossyUTF8(stdout.Bytes()), "\x00") {
		if path != "" {
			files = append(files, path)
		}
	}
	return files, nil
}

func auditPath(path string) (string, bool) {
	normalized := strings.ReplaceAll(path, `\`, "/")
	name := normalized[strings.LastIndexByte(normalized, '/')+1:]
	for _, denied := range []string{"AGENTS.md", "plugroot.local.toml", ".env", ".env.local", "id_rsa", "id_ed25519"} {
		if normalized == denied || name == denied {
			return "private file path is tracked", true
		}
	}
	if strings.HasPrefix(name, ".env.") && name != ".env.example" {
		return "private env file is tracked", true
	}
	for _, prefix := range []string{"docs/private/", ".plugroot/", "repos/", "backups/"} {
		if strings.HasPrefix(normalized, prefix) {
			return "ignored private/runtime path is tracked", true
		}
	}
	for _, part := range []string{"/data/", "/cache/", "/config/"} {
		if strings.Contains(normalized, part) {
			return "service runtime data path is tracked", true
		}
	}
	if ext, ok := extension(name); ok {
		switch ext {
		case "db", "sqlite", "sqlite3", "log", "pid", "pem", "key", "p12", "pfx":
			return "private state or key-like file is tracked", true
		}
	}
	return "", false
}

func loadAuditDenylist(root string) []string {
	paths := []string{
		joinPath(root, "docs/private/audit-denylist.txt"),
		joinPath(root, ".plugroot/audit-denylist.txt"),
	}
	stateRoot, ok := os.LookupEnv("PLUGROOT_STATE_ROOT")
	if !ok {
		stateRoot = defaultStateRoot(root)
	}
	paths = append(paths, joinPath(stateRoot, ".plugroot/audit-denylist.txt"))
	if extra, ok := os.LookupEnv("PLUGROOT_AUDIT_DENYLIST"); ok {
		paths = append(paths, extra)
	}
	seen := map[string]bool{}
	var terms []string
	for _, path := range paths {
		text, err := readText(path)
		if err != nil {
			continue
		}
		for _, line := range rustLines(text) {
			term := strings.TrimSpace(line)
			if term == "" || strings.HasPrefix(term, "#") {
				continue
			}
			term = asciiLower(term)
			if !seen[term] {
				seen[term] = true
				terms = append(terms, term)
			}
		}
	}
	sort.Strings(terms)
	return terms
}

func denylistIdentifierByte(c byte) bool {
	return c == '_' || c == '-' || '0' <= c && c <= '9' || 'a' <= c && c <= 'z' || 'A' <= c && c <= 'Z'
}

func containsDenylistTerm(line, term string) bool {
	if term == "" {
		return false
	}
	lower := asciiLower(line)
	for i := 0; i < len(term); i++ {
		if !denylistIdentifierByte(term[i]) {
			return strings.Contains(lower, term)
		}
	}
	offset := 0
	for {
		relative := strings.Index(lower[offset:], term)
		if relative < 0 {
			return false
		}
		start := offset + relative
		end := start + len(term)
		// A multi-byte character next to the term is not an identifier
		// character, so looking at single bytes is enough.
		left := start > 0 && denylistIdentifierByte(lower[start-1])
		right := end < len(lower) && denylistIdentifierByte(lower[end])
		if !left && !right {
			return true
		}
		offset = end
	}
}

func auditLine(line string, terms []string) []string {
	var findings []string
	if strings.HasPrefix(strings.TrimLeftFunc(line, isRustWhitespace), "#") {
		return findings
	}
	if strings.Contains(line, "-----BEGIN ") && strings.Contains(line, "PRIVATE"+" KEY") {
		findings = append(findings, "private key material")
	}
	for _, marker := range tokenMarkers() {
		if containsTokenMarker(line, marker) {
			findings = append(findings, "token marker `"+marker+"`")
		}
	}
	if containsTailscaleIPv4(line) {
		findings = append(findings, "Tailscale/CGNAT private IP address")
	}
	if suspiciousSecretAssignment(line) {
		findings = append(findings, "non-placeholder secret-looking assignment")
	}
	for _, term := range terms {
		if containsDenylistTerm(line, term) {
			findings = append(findings, "local denylist term")
		}
	}
	return findings
}

// tokenMarkers are split so this file does not match itself.
func tokenMarkers() []string {
	return []string{
		"github" + "_pat_", "gh" + "p_", "gh" + "o_", "gh" + "s_", "gh" + "u_",
		"s" + "k-", "xo" + "xb-", "xo" + "xp-",
	}
}

func containsTokenMarker(line, marker string) bool {
	offset := 0
	for {
		relative := strings.Index(line[offset:], marker)
		if relative < 0 {
			return false
		}
		suffixStart := offset + relative + len(marker)
		n := 0
		for i := suffixStart; i < len(line) && denylistIdentifierByte(line[i]); i++ {
			n++
		}
		if n >= tokenMarkerMinSuffix(marker) {
			return true
		}
		offset = suffixStart
	}
}

func tokenMarkerMinSuffix(marker string) int {
	switch marker {
	case "s" + "k-", "xo" + "xb-", "xo" + "xp-":
		return 16
	}
	return 12
}

func suspiciousSecretAssignment(line string) bool {
	key, value, ok := strings.Cut(line, "=")
	if !ok {
		if key, value, ok = strings.Cut(line, ":"); !ok {
			return false
		}
	}
	key = asciiLower(strings.TrimFunc(key, isRustWhitespace))
	if key == "" || len(key) > 80 {
		return false
	}
	for i := 0; i < len(key); i++ {
		c := key[i]
		if !(denylistIdentifierByte(c) || c == '.') {
			return false
		}
	}
	found := false
	for _, needle := range []string{"password", "passwd", "secret", "token", "api_key", "apikey", "access_key", "client_secret", "private_key"} {
		if strings.Contains(key, needle) {
			found = true
			break
		}
	}
	if !found {
		return false
	}
	value = strings.TrimFunc(value, isRustWhitespace)
	value = strings.Trim(value, `"`)
	value = strings.Trim(value, "'")
	value = strings.TrimRight(value, ",")
	value = strings.TrimFunc(value, isRustWhitespace)
	switch asciiLower(value) {
	case "", "example", "placeholder", "changeme", "change-me", "redacted", "dummy", "none", "null", "false", "true":
		return false
	}
	return !(strings.HasPrefix(value, "${") || strings.HasPrefix(value, "<") || strings.HasPrefix(value, "your-"))
}

func containsTailscaleIPv4(line string) bool {
	tokens := strings.FieldsFunc(line, func(r rune) bool { return !(r >= '0' && r <= '9' || r == '.') })
	for _, token := range tokens {
		if strings.Count(token, ".") != 3 {
			continue
		}
		var octets []uint64
		for _, part := range strings.Split(token, ".") {
			if n, err := strconv.ParseUint(part, 10, 16); err == nil {
				octets = append(octets, n)
			}
		}
		if len(octets) != 4 || octets[0] != 100 || octets[1] < 64 || octets[1] > 127 {
			continue
		}
		if octets[2] <= 255 && octets[3] <= 255 {
			return true
		}
	}
	return false
}
