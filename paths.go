package main

// Paths as the Rust version handled them (std::path on Unix), so printed
// paths stay the same: joining never cleans, comparisons go by component,
// and cleanPath resolves "." and ".." lexically.

import (
	"os"
	"strings"
)

func isAbs(path string) bool { return strings.HasPrefix(path, "/") }

// joinPath is Rust's Path::join: an absolute rest replaces the base,
// otherwise one separator is added unless the base already ends in one.
func joinPath(base, rest string) string {
	if isAbs(rest) {
		return rest
	}
	if base == "" || strings.HasSuffix(base, "/") {
		return base + rest
	}
	return base + "/" + rest
}

// components splits a path the way Rust does: "/" first for an absolute
// path, empty parts and inner "." dropped, a leading "." kept for a
// relative path.
func components(path string) []string {
	var out []string
	rest := path
	if isAbs(path) {
		out = append(out, "/")
		rest = strings.TrimLeft(path, "/")
	} else if path == "." || strings.HasPrefix(path, "./") {
		out = append(out, ".")
		rest = strings.TrimLeft(strings.TrimPrefix(path, "."), "/")
	}
	for _, part := range strings.Split(rest, "/") {
		if part == "" || part == "." {
			continue
		}
		out = append(out, part)
	}
	return out
}

// startsWithPath is Rust's Path::starts_with: whole components.
func startsWithPath(path, base string) bool {
	p, b := components(path), components(base)
	if len(b) > len(p) {
		return false
	}
	for i := range b {
		if p[i] != b[i] {
			return false
		}
	}
	return true
}

func samePath(a, b string) bool {
	return startsWithPath(a, b) && len(components(a)) == len(components(b))
}

// parentPath is Rust's Path::parent; ok is false for "/" and "".
func parentPath(path string) (string, bool) {
	comps := components(path)
	if len(comps) == 0 || len(comps) == 1 && comps[0] == "/" {
		return "", false
	}
	trimmed := strings.TrimRight(path, "/")
	i := strings.LastIndexByte(trimmed, '/')
	if i < 0 {
		return "", true
	}
	if i == 0 {
		return "/", true
	}
	parent := strings.TrimRight(trimmed[:i], "/")
	if parent == "" {
		return "/", true
	}
	// "a/./b": Rust's parent drops the trailing "." component too.
	for strings.HasSuffix(parent, "/.") {
		parent = strings.TrimRight(strings.TrimSuffix(parent, "/."), "/")
	}
	return parent, true
}

// fileName is Rust's Path::file_name; ok is false for "/", "" and "..".
func fileName(path string) (string, bool) {
	comps := components(path)
	if len(comps) == 0 {
		return "", false
	}
	last := comps[len(comps)-1]
	if last == "/" || last == ".." || last == "." {
		return "", false
	}
	return last, true
}

// extension is Rust's Path::extension of a file name: "" with ok false for
// "name" and ".name".
func extension(name string) (string, bool) {
	if name == ".." {
		return "", false
	}
	i := strings.LastIndexByte(name, '.')
	if i <= 0 {
		return "", false
	}
	return name[i+1:], true
}

// cleanPath resolves "." and ".." without touching the file system.
func cleanPath(path string) string {
	rooted := false
	var parts []string
	for _, c := range components(path) {
		switch c {
		case "/":
			rooted, parts = true, nil
		case ".":
		case "..":
			if len(parts) > 0 {
				parts = parts[:len(parts)-1]
			}
		default:
			parts = append(parts, c)
		}
	}
	if rooted {
		return "/" + strings.Join(parts, "/")
	}
	return strings.Join(parts, "/")
}

func sameOrInside(parent, child string) bool {
	parent, child = cleanPath(parent), cleanPath(child)
	return child == parent || startsWithPath(child, parent)
}

func resolvePath(root, value string) string {
	if isAbs(value) {
		return value
	}
	return joinPath(root, value)
}

func stateRootForManifest(codeRoot string, m *Manifest) string {
	if m.Plugroot != nil {
		if m.Plugroot.StateRoot != nil {
			return resolvePath(codeRoot, *m.Plugroot.StateRoot)
		}
		if m.Plugroot.Root != nil {
			return resolvePath(codeRoot, *m.Plugroot.Root)
		}
	}
	return defaultStateRoot(codeRoot)
}

func defaultStateRoot(codeRoot string) string {
	clean := cleanPath(codeRoot)
	name, ok := fileName(clean)
	if !ok || strings.TrimSpace(name) == "" {
		name = "plugroot"
	}
	parent, _ := parentPath(clean)
	return joinPath(parent, name+"-state")
}

// nearestGitRoot walks up from path to the first directory holding .git.
func nearestGitRoot(path string) (string, bool) {
	current := path
	if !isDir(path) {
		if parent, ok := parentPath(path); ok {
			current = parent
		}
	}
	for {
		if exists(joinPath(current, ".git")) {
			return cleanPath(current), true
		}
		parent, ok := parentPath(current)
		if !ok {
			return "", false
		}
		current = parent
	}
}

// stripPrefixPath is Rust's Path::strip_prefix: the rest after whole
// components, ok false when base is not a prefix.
func stripPrefixPath(path, base string) (string, bool) {
	if !startsWithPath(path, base) {
		return "", false
	}
	p := components(path)
	rest := p[len(components(base)):]
	return strings.Join(rest, "/"), true
}

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func isDir(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

func isFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}

// absoluteCleanPath makes path absolute against the working directory and
// cleans it.
func absoluteCleanPath(path string) (string, error) {
	if !isAbs(path) {
		cwd, err := os.Getwd()
		if err != nil {
			return "", ioError(err)
		}
		path = joinPath(cwd, path)
	}
	return cleanPath(path), nil
}

// tarRelativePath drops the leading "/" of a cleaned path; ok is false for
// the file-system root.
func tarRelativePath(path string) (string, bool) {
	clean := cleanPath(path)
	if isAbs(clean) {
		text := strings.TrimPrefix(clean, "/")
		return text, text != ""
	}
	return clean, true
}
