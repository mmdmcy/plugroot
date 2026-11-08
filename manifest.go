package main

// The manifest (plugroot.toml plus the private overlays), the environment
// files and the application context.

import (
	"fmt"
	"os"
	"strings"
)

type Manifest struct {
	Plugroot  *PlugrootConfig
	Host      *HostConfig
	Repo      []Repo
	Directory []ManagedDir
	File      []ManagedFile
	Symlink   []ManagedSymlink
	Service   []Service
}

type PlugrootConfig struct {
	CodeRoot  *string
	Root      *string
	StateRoot *string
	RepoDir   *string
}

type HostConfig struct {
	Name      *string
	PrivateIP *string
	User      *string
}

type Repo struct {
	ID     string
	URL    string
	Path   string
	GitRef *string
}

type ManagedDir struct {
	Path  string
	Owner *string
	Mode  *string
}

type ManagedFile struct {
	Source string
	Target string
	Owner  *string
	Mode   *string
}

type ManagedSymlink struct {
	Source string
	Target string
}

type Service struct {
	ID          string
	Name        string
	Plane       *string
	Category    *string
	Kind        string
	Path        *string
	Unit        *string
	User        *string
	Host        *string
	Port        *uint16
	URL         *string
	Access      *string
	Description *string
	Containers  []string
	Controls    []string
	Ports       []string
	Optional    *bool
	Repo        *string
	WorkingDir  *string
	// Command is nil when the manifest has none; an empty list is a list.
	Command    *[]string
	Env        []string
	EnvFile    *string
	UnitSource *string
}

// Context is a loaded code root: its manifest and every environment value
// (the process's, then the code root's .env, then the state root's).
type Context struct {
	Root      string
	Manifest  Manifest
	EnvValues map[string]string
}

func loadContext(root string) (*Context, error) {
	merged := map[string]string{}
	for _, kv := range os.Environ() {
		if key, value, ok := strings.Cut(kv, "="); ok {
			merged[key] = value
		}
	}
	for key, value := range loadEnvFile(joinPath(root, ".env")) {
		merged[key] = value
	}
	mainPath := joinPath(root, "plugroot.toml")
	manifest, err := loadManifestFile(mainPath, merged)
	if err != nil {
		return nil, err
	}
	stateRoot := stateRootForManifest(root, manifest)
	for key, value := range loadEnvFile(joinPath(stateRoot, ".env")) {
		merged[key] = value
	}
	if manifest, err = loadManifestFile(mainPath, merged); err != nil {
		return nil, err
	}
	stateRoot = stateRootForManifest(root, manifest)
	for _, local := range []string{joinPath(root, "plugroot.local.toml"), joinPath(stateRoot, "plugroot.local.toml")} {
		if exists(local) {
			overlay, err := loadManifestFile(local, merged)
			if err != nil {
				return nil, err
			}
			manifest = mergeManifest(manifest, overlay)
		}
	}
	return &Context{Root: root, Manifest: *manifest, EnvValues: merged}, nil
}

func (c *Context) service(id string) (Service, bool) {
	for _, s := range c.Manifest.Service {
		if s.ID == id {
			return s, true
		}
	}
	return Service{}, false
}

// servicesForTarget is one service by id, or with "all" every service
// that has controls.
func (c *Context) servicesForTarget(target string) []Service {
	if target != "all" {
		if s, ok := c.service(target); ok {
			return []Service{s}
		}
		return nil
	}
	var out []Service
	for _, s := range c.Manifest.Service {
		if len(s.Controls) > 0 {
			out = append(out, s)
		}
	}
	return out
}

func (c *Context) stateRoot() string { return stateRootForManifest(c.Root, &c.Manifest) }

func (c *Context) repoDir() (string, bool) {
	if c.Manifest.Plugroot == nil || c.Manifest.Plugroot.RepoDir == nil {
		return "", false
	}
	return resolvePath(c.Root, *c.Manifest.Plugroot.RepoDir), true
}

func (c *Context) hostField(get func(*HostConfig) *string) *string {
	if c.Manifest.Host == nil {
		return nil
	}
	return get(c.Manifest.Host)
}

// loadEnvFile reads KEY=VALUE lines; a missing or unreadable file is empty.
func loadEnvFile(path string) map[string]string {
	values := map[string]string{}
	raw, err := readText(path)
	if err != nil {
		return values
	}
	for _, rawLine := range rustLines(raw) {
		line := strings.TrimSpace(rawLine)
		if line == "" || strings.HasPrefix(line, "#") || !strings.Contains(line, "=") {
			continue
		}
		key, value, _ := strings.Cut(line, "=")
		value = strings.Trim(strings.Trim(strings.TrimSpace(value), `"`), "'")
		values[strings.TrimSpace(key)] = value
	}
	return values
}

func loadManifestFile(path string, env map[string]string) (*Manifest, error) {
	raw, err := readText(path)
	if err != nil {
		return nil, err
	}
	table, err := parseTOML(expandVars(raw, env))
	if err != nil {
		return nil, fmt.Errorf("TOML parse error at %w", err)
	}
	return decodeManifest(table)
}

// expandVars replaces ${NAME} and ${NAME:-default}; an unknown ${NAME}
// stays as it is.
func expandVars(input string, values map[string]string) string {
	var b strings.Builder
	for i := 0; i < len(input); {
		if strings.HasPrefix(input[i:], "${") {
			if end := strings.IndexByte(input[i+2:], '}'); end >= 0 {
				body := input[i+2 : i+2+end]
				if replacement, ok := expandVarBody(body, values); ok {
					b.WriteString(replacement)
				} else {
					b.WriteString("${" + body + "}")
				}
				i += end + 3
				continue
			}
		}
		b.WriteByte(input[i])
		i++
	}
	return b.String()
}

func expandVarBody(body string, values map[string]string) (string, bool) {
	if key, fallback, ok := strings.Cut(body, ":-"); ok {
		if value := values[key]; value != "" {
			return value, true
		}
		return fallback, true
	}
	value, ok := values[body]
	return value, ok
}

func mergeManifest(base, overlay *Manifest) *Manifest {
	if overlay.Plugroot != nil {
		base.Plugroot = overlay.Plugroot
	}
	if overlay.Host != nil {
		base.Host = overlay.Host
	}
	base.Repo = mergeByID(base.Repo, overlay.Repo, func(r Repo) string { return r.ID })
	base.Directory = mergeByID(base.Directory, overlay.Directory, func(d ManagedDir) string { return d.Path })
	base.File = mergeByID(base.File, overlay.File, func(f ManagedFile) string { return f.Target })
	base.Symlink = mergeByID(base.Symlink, overlay.Symlink, func(s ManagedSymlink) string { return s.Target })
	base.Service = mergeByID(base.Service, overlay.Service, func(s Service) string { return s.ID })
	return base
}

// mergeByID replaces the first entry with the same id, or appends.
func mergeByID[T any](base, overlay []T, id func(T) string) []T {
	for _, item := range overlay {
		replaced := false
		for i := range base {
			if id(base[i]) == id(item) {
				base[i] = item
				replaced = true
				break
			}
		}
		if !replaced {
			base = append(base, item)
		}
	}
	return base
}

// decodeManifest reads the manifest's fields from a TOML table as the Rust
// version's serde did: unknown keys ignored, required fields required and
// every field of its type.
func decodeManifest(table map[string]any) (*Manifest, error) {
	d := &fieldDecoder{}
	m := &Manifest{}
	if t := d.table(table, "plugroot", "plugroot"); t != nil {
		m.Plugroot = &PlugrootConfig{
			CodeRoot:  d.optText(t, "code_root", "plugroot"),
			Root:      d.optText(t, "root", "plugroot"),
			StateRoot: d.optText(t, "state_root", "plugroot"),
			RepoDir:   d.optText(t, "repo_dir", "plugroot"),
		}
	}
	if t := d.table(table, "host", "host"); t != nil {
		m.Host = &HostConfig{
			Name:      d.optText(t, "name", "host"),
			PrivateIP: d.optText(t, "private_ip", "host"),
			User:      d.optText(t, "user", "host"),
		}
	}
	for i, t := range d.tables(table, "repo") {
		where := fmt.Sprintf("repo[%d]", i)
		m.Repo = append(m.Repo, Repo{
			ID: d.text(t, "id", where), URL: d.text(t, "url", where), Path: d.text(t, "path", where),
			GitRef: d.optText(t, "ref", where),
		})
	}
	for i, t := range d.tables(table, "directory") {
		where := fmt.Sprintf("directory[%d]", i)
		m.Directory = append(m.Directory, ManagedDir{
			Path: d.text(t, "path", where), Owner: d.optText(t, "owner", where), Mode: d.optText(t, "mode", where),
		})
	}
	for i, t := range d.tables(table, "file") {
		where := fmt.Sprintf("file[%d]", i)
		m.File = append(m.File, ManagedFile{
			Source: d.text(t, "source", where), Target: d.text(t, "target", where),
			Owner: d.optText(t, "owner", where), Mode: d.optText(t, "mode", where),
		})
	}
	for i, t := range d.tables(table, "symlink") {
		where := fmt.Sprintf("symlink[%d]", i)
		m.Symlink = append(m.Symlink, ManagedSymlink{Source: d.text(t, "source", where), Target: d.text(t, "target", where)})
	}
	for i, t := range d.tables(table, "service") {
		where := fmt.Sprintf("service[%d]", i)
		s := Service{
			ID: d.text(t, "id", where), Name: d.text(t, "name", where),
			Plane: d.optText(t, "plane", where), Category: d.optText(t, "category", where),
			Kind: d.text(t, "kind", where), Path: d.optText(t, "path", where),
			Unit: d.optText(t, "unit", where), User: d.optText(t, "user", where),
			Host: d.optText(t, "host", where), Port: d.optPort(t, "port", where),
			URL: d.optText(t, "url", where), Access: d.optText(t, "access", where),
			Description: d.optText(t, "description", where),
			Containers:  d.optTexts(t, "containers", where), Controls: d.optTexts(t, "controls", where),
			Ports: d.optTexts(t, "ports", where), Optional: d.optBool(t, "optional", where),
			Repo: d.optText(t, "repo", where), WorkingDir: d.optText(t, "working_dir", where),
			Env: d.optTexts(t, "env", where), EnvFile: d.optText(t, "env_file", where),
			UnitSource: d.optText(t, "unit_source", where),
		}
		if _, given := t["command"]; given {
			command := d.optTexts(t, "command", where)
			if command == nil {
				command = []string{}
			}
			s.Command = &command
		}
		m.Service = append(m.Service, s)
	}
	if d.err != nil {
		return nil, d.err
	}
	return m, nil
}

// fieldDecoder keeps the first error.
type fieldDecoder struct{ err error }

func (d *fieldDecoder) fail(where, key, expected string) {
	if d.err == nil {
		d.err = fmt.Errorf("invalid manifest: %s.%s: expected %s", where, key, expected)
	}
}

func (d *fieldDecoder) table(t map[string]any, key, where string) map[string]any {
	value, given := t[key]
	if !given {
		return nil
	}
	table, ok := value.(map[string]any)
	if !ok {
		d.err = firstError(d.err, fmt.Errorf("invalid manifest: %s: expected a table", where))
		return nil
	}
	return table
}

func (d *fieldDecoder) tables(t map[string]any, key string) []map[string]any {
	value, given := t[key]
	if !given {
		return nil
	}
	list, ok := value.([]any)
	if !ok {
		d.err = firstError(d.err, fmt.Errorf("invalid manifest: %s: expected an array of tables", key))
		return nil
	}
	out := make([]map[string]any, 0, len(list))
	for i, item := range list {
		table, ok := item.(map[string]any)
		if !ok {
			d.err = firstError(d.err, fmt.Errorf("invalid manifest: %s[%d]: expected a table", key, i))
			return nil
		}
		out = append(out, table)
	}
	return out
}

func firstError(old, new error) error {
	if old != nil {
		return old
	}
	return new
}

func (d *fieldDecoder) text(t map[string]any, key, where string) string {
	value, given := t[key]
	if !given {
		if d.err == nil {
			d.err = fmt.Errorf("invalid manifest: %s: missing field `%s`", where, key)
		}
		return ""
	}
	s, ok := value.(string)
	if !ok {
		d.fail(where, key, "a string")
	}
	return s
}

func (d *fieldDecoder) optText(t map[string]any, key, where string) *string {
	value, given := t[key]
	if !given {
		return nil
	}
	s, ok := value.(string)
	if !ok {
		d.fail(where, key, "a string")
		return nil
	}
	return &s
}

func (d *fieldDecoder) optTexts(t map[string]any, key, where string) []string {
	value, given := t[key]
	if !given {
		return nil
	}
	list, ok := value.([]any)
	if !ok {
		d.fail(where, key, "a list of strings")
		return nil
	}
	out := make([]string, 0, len(list))
	for _, item := range list {
		s, ok := item.(string)
		if !ok {
			d.fail(where, key, "a list of strings")
			return nil
		}
		out = append(out, s)
	}
	return out
}

func (d *fieldDecoder) optBool(t map[string]any, key, where string) *bool {
	value, given := t[key]
	if !given {
		return nil
	}
	b, ok := value.(bool)
	if !ok {
		d.fail(where, key, "true or false")
		return nil
	}
	return &b
}

func (d *fieldDecoder) optPort(t map[string]any, key, where string) *uint16 {
	value, given := t[key]
	if !given {
		return nil
	}
	n, ok := value.(int64)
	if !ok || n < 0 || n > 65535 {
		d.fail(where, key, "an integer from 0 to 65535")
		return nil
	}
	port := uint16(n)
	return &port
}

func deref(s *string, fallback string) string {
	if s == nil {
		return fallback
	}
	return *s
}
