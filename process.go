package main

// External commands, and errors worded as the Rust version printed them.

import (
	"bytes"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"syscall"
	"unicode/utf8"
)

// CmdOutput is a finished command: its exit code (127 when it could not
// start) and its standard output then error, trimmed.
type CmdOutput struct {
	Code int
	Text string
}

// ioError words an operating-system error the way Rust's io::Error did:
// "No such file or directory (os error 2)". Other errors pass unchanged.
func ioError(err error) error {
	if err == nil {
		return nil
	}
	if errors.Is(err, exec.ErrNotFound) {
		return errnoError(syscall.ENOENT)
	}
	var errno syscall.Errno
	if errors.As(err, &errno) {
		return errnoError(errno)
	}
	return err
}

func errnoError(errno syscall.Errno) error {
	text := errno.Error()
	if text != "" {
		text = strings.ToUpper(text[:1]) + text[1:]
	}
	return fmt.Errorf("%s (os error %d)", text, int(errno))
}

// errorKind words an I/O error's kind the way Rust's ErrorKind displayed
// it in a port check: "connection refused", "timed out".
func errorKind(err error) string {
	var netErr interface{ Timeout() bool }
	if errors.As(err, &netErr) && netErr.Timeout() {
		return "timed out"
	}
	var errno syscall.Errno
	if !errors.As(err, &errno) {
		return "other error"
	}
	switch errno {
	case syscall.ECONNREFUSED:
		return "connection refused"
	case syscall.ECONNRESET:
		return "connection reset"
	case syscall.EHOSTUNREACH:
		return "host unreachable"
	case syscall.ENETUNREACH:
		return "network unreachable"
	case syscall.ETIMEDOUT:
		return "timed out"
	case syscall.EACCES, syscall.EPERM:
		return "permission denied"
	case syscall.EADDRNOTAVAIL:
		return "address not available"
	case syscall.EADDRINUSE:
		return "address in use"
	case syscall.ENETDOWN:
		return "network down"
	case syscall.ECONNABORTED:
		return "connection aborted"
	case syscall.EINVAL:
		return "invalid input parameter"
	}
	return "uncategorized error"
}

// lossyUTF8 is Rust's String::from_utf8_lossy: each maximal invalid
// sequence becomes one U+FFFD.
func lossyUTF8(data []byte) string {
	if utf8.Valid(data) {
		return string(data)
	}
	var b strings.Builder
	for i := 0; i < len(data); {
		r, size := utf8.DecodeRune(data[i:])
		if r != utf8.RuneError || size > 1 {
			b.Write(data[i : i+size])
			i += size
			continue
		}
		b.WriteRune(utf8.RuneError)
		i += invalidPrefixLength(data[i:])
	}
	return b.String()
}

// invalidPrefixLength is how many bytes one replacement character stands
// for: the lead byte and the continuation bytes that could still have
// formed a character with it.
func invalidPrefixLength(b []byte) int {
	lead := b[0]
	var need int
	lo, hi := byte(0x80), byte(0xBF)
	switch {
	case lead >= 0xC2 && lead <= 0xDF:
		need = 1
	case lead == 0xE0:
		need, lo = 2, 0xA0
	case lead >= 0xE1 && lead <= 0xEC, lead == 0xEE, lead == 0xEF:
		need = 2
	case lead == 0xED:
		need, hi = 2, 0x9F
	case lead == 0xF0:
		need, lo = 3, 0x90
	case lead >= 0xF1 && lead <= 0xF3:
		need = 3
	case lead == 0xF4:
		need, hi = 3, 0x8F
	default:
		return 1
	}
	n := 1
	for k := 0; k < need && n < len(b); k++ {
		c := b[n]
		if k == 0 && (c < lo || c > hi) || k > 0 && (c < 0x80 || c > 0xBF) {
			break
		}
		n++
	}
	return n
}

// lookPathIn finds a program in a search path, the way Rust found a command
// whose PATH it had set.
func lookPathIn(name, searchPath string) string {
	if strings.Contains(name, "/") {
		return name
	}
	for _, dir := range strings.Split(searchPath, ":") {
		candidate := joinPath(dir, name)
		info, err := os.Stat(candidate)
		if err == nil && info.Mode().IsRegular() && info.Mode().Perm()&0o111 != 0 {
			return candidate
		}
	}
	return name
}

// runCommand runs a program with extra environment values and returns its
// combined, trimmed output.
func runCommand(program string, args []string, cwd string, envs [][2]string) CmdOutput {
	cmd := exec.Command(program, args...)
	if cwd != "" {
		cmd.Dir = cwd
	}
	if len(envs) > 0 {
		cmd.Env = os.Environ()
		for _, kv := range envs {
			cmd.Env = append(cmd.Env, kv[0]+"="+kv[1])
			if kv[0] == "PATH" {
				cmd.Path = lookPathIn(program, kv[1])
				cmd.Err = nil
			}
		}
	}
	return collectOutput(cmd)
}

// runCommandWithEnvValues runs a program with the given environment added
// to the process's own.
func runCommandWithEnvValues(program string, args []string, cwd string, envs map[string]string) CmdOutput {
	cmd := exec.Command(program, args...)
	if cwd != "" {
		cmd.Dir = cwd
	}
	cmd.Env = os.Environ()
	for key, value := range envs {
		cmd.Env = append(cmd.Env, key+"="+value)
	}
	if path, ok := envs["PATH"]; ok {
		cmd.Path = lookPathIn(program, path)
		cmd.Err = nil
	}
	return collectOutput(cmd)
}

func runPrefixedCommand(prefix []string, program string, args []string) CmdOutput {
	if len(prefix) == 0 {
		return runCommand(program, args, "", nil)
	}
	full := append(append(append([]string{}, prefix[1:]...), program), args...)
	return runCommand(prefix[0], full, "", nil)
}

func collectOutput(cmd *exec.Cmd) CmdOutput {
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()
	var exit *exec.ExitError
	if err != nil && !errors.As(err, &exit) {
		return CmdOutput{Code: 127, Text: startError(err)}
	}
	code := 0
	if exit != nil {
		code = exit.ExitCode()
		if code < 0 {
			code = 1
		}
	}
	text := lossyUTF8(stdout.Bytes()) + lossyUTF8(stderr.Bytes())
	return CmdOutput{Code: code, Text: strings.TrimSpace(text)}
}

// startError is why a program did not start, as Rust worded it.
func startError(err error) string {
	var pathErr *fs.PathError
	if errors.As(err, &pathErr) {
		return ioError(pathErr.Err).Error()
	}
	return ioError(err).Error()
}

func ensureSuccess(out CmdOutput) error {
	if out.Code == 0 {
		return nil
	}
	return errors.New(out.Text)
}

func printCmdOutput(out CmdOutput) {
	if out.Text != "" {
		fmt.Println(out.Text)
	}
}

// commandText is a successful command's trimmed standard output.
func commandText(args ...string) (string, bool) {
	cmd := exec.Command(args[0], args[1:]...)
	var stdout bytes.Buffer
	cmd.Stdout = &stdout
	if err := cmd.Run(); err != nil {
		return "", false
	}
	return strings.TrimSpace(lossyUTF8(stdout.Bytes())), true
}

func isRoot() bool { return os.Geteuid() == 0 }

func userHome(user string) (string, bool) {
	line, ok := commandText("getent", "passwd", user)
	if !ok {
		return "", false
	}
	fields := strings.Split(line, ":")
	if len(fields) < 6 || fields[5] == "" {
		return "", false
	}
	return fields[5], true
}

// userSystemdPrefix is the command prefix that reaches user's systemd
// manager: env with its runtime directory, through runuser when that user
// is not the current one.
func userSystemdPrefix(user string) ([]string, bool) {
	uid, ok := commandText("id", "-u", user)
	if !ok {
		return nil, false
	}
	home, ok := userHome(user)
	if !ok {
		home = "/home/" + user
	}
	envArgs := []string{
		"env",
		"XDG_RUNTIME_DIR=/run/user/" + uid,
		"DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/" + uid + "/bus",
		"HOME=" + home,
	}
	if strconv.Itoa(os.Geteuid()) == uid {
		return envArgs, true
	}
	return append([]string{"runuser", "-u", user, "--"}, envArgs...), true
}

// gitDirtyDetails is `git status --porcelain` for a checkout, at most 20
// lines; "" when clean or not in a checkout.
func gitDirtyDetails(path string) (string, error) {
	if _, ok := nearestGitRoot(path); !ok {
		return "", nil
	}
	out := runCommand("git", []string{"-C", path, "status", "--porcelain=v1"}, "", nil)
	if out.Code != 0 {
		return "", errors.New(out.Text)
	}
	var lines []string
	for _, line := range rustLines(out.Text) {
		if strings.TrimSpace(line) != "" {
			lines = append(lines, line)
		}
	}
	if len(lines) == 0 {
		return "", nil
	}
	return limitLines(lines, 20), nil
}

// rustLines splits like Rust's str::lines: on \n, dropping one \r before
// it, with no empty line after a final newline.
func rustLines(text string) []string {
	if text == "" {
		return nil
	}
	lines := strings.Split(text, "\n")
	if lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	for i, line := range lines {
		lines[i] = strings.TrimSuffix(line, "\r")
	}
	return lines
}

func limitLines(lines []string, limit int) string {
	selected := lines
	if len(lines) > limit {
		selected = append(append([]string{}, lines[:limit]...), fmt.Sprintf("... %d more", len(lines)-limit))
	}
	return strings.Join(selected, "\n")
}

// asciiLower lowercases A-Z only, as Rust's to_ascii_lowercase.
func asciiLower(s string) string {
	b := []byte(s)
	for i, c := range b {
		if 'A' <= c && c <= 'Z' {
			b[i] = c + 32
		}
	}
	return string(b)
}

// readText reads a UTF-8 text file, refusing other bytes as Rust's
// read_to_string did.
func readText(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", ioError(err)
	}
	if !utf8.Valid(data) {
		return "", errors.New("stream did not contain valid UTF-8")
	}
	return string(data), nil
}

// parseUnsigned parses like Rust's unsigned from_str: decimal digits with
// one optional leading +.
func parseUnsigned(s string, bits int) (uint64, bool) {
	s = strings.TrimPrefix(s, "+")
	if s == "" || s[0] == '+' || s[0] == '-' {
		return 0, false
	}
	n, err := strconv.ParseUint(s, 10, bits)
	return n, err == nil
}
