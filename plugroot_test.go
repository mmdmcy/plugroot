package main

// The Rust version's 46 tests, carried over one for one (same names in Go
// case); the reader and path tests are in reader_test.go.

import (
	"encoding/hex"
	"errors"
	"fmt"
	"math"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func strp(s string) *string { return &s }

func writeFile(t *testing.T, path, text string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
		t.Fatal(err)
	}
}

func mustContext(t *testing.T, root string) *Context {
	t.Helper()
	ctx, err := loadContext(root)
	if err != nil {
		t.Fatal(err)
	}
	return ctx
}

func errKindOf(err error) errKind {
	var k kindError
	if errors.As(err, &k) {
		return k.kind
	}
	return 0
}

// --- app.rs ---

func TestExpandsVarsWithDefaults(t *testing.T) {
	got := expandVars("${SET} ${MISSING:-fallback} ${UNKNOWN}", map[string]string{"SET": "value"})
	if got != "value fallback ${UNKNOWN}" {
		t.Fatal(got)
	}
}

func TestParsesMinimalManifest(t *testing.T) {
	raw := `
[plugroot]
root = "/srv/plugroot"
state_root = "/var/lib/plugroot"

[[repo]]
id = "example"
url = "https://example.invalid/repo.git"
path = "/tmp/example"

[[symlink]]
source = "/var/lib/plugroot/workspaces/example/.env"
target = "/tmp/example/.env"

[[service]]
id = "example"
name = "Example"
kind = "port"
host = "127.0.0.1"
port = 9
`
	table, err := parseTOML(raw)
	if err != nil {
		t.Fatal(err)
	}
	m, err := decodeManifest(table)
	if err != nil {
		t.Fatal(err)
	}
	if deref(m.Plugroot.StateRoot, "") != "/var/lib/plugroot" || len(m.Repo) != 1 || len(m.Symlink) != 1 ||
		m.Symlink[0].Target != "/tmp/example/.env" || m.Service[0].ID != "example" {
		t.Fatalf("%+v", m)
	}
}

func TestStateRootPrefersStateRootOverLegacyRoot(t *testing.T) {
	m := &Manifest{Plugroot: &PlugrootConfig{Root: strp("/legacy"), StateRoot: strp("/private-state")}}
	if got := stateRootForManifest("/code", m); got != "/private-state" {
		t.Fatal(got)
	}
}

func TestStateRootDefaultsToASiblingDirectory(t *testing.T) {
	if got := defaultStateRoot("/srv/plugroot"); got != "/srv/plugroot-state" {
		t.Fatal(got)
	}
}

func TestContextLoadsStateRootOverlay(t *testing.T) {
	code, state := t.TempDir(), t.TempDir()
	writeFile(t, filepath.Join(code, "plugroot.toml"), fmt.Sprintf("\n[plugroot]\nstate_root = %q\n\n[[service]]\nid = \"alpha\"\nname = \"Alpha\"\nkind = \"noop\"\n", state))
	writeFile(t, filepath.Join(state, "plugroot.local.toml"), "\n[[service]]\nid = \"beta\"\nname = \"Beta\"\nkind = \"noop\"\n")
	ctx := mustContext(t, code)
	if _, ok := ctx.service("alpha"); !ok {
		t.Fatal("alpha")
	}
	if _, ok := ctx.service("beta"); !ok {
		t.Fatal("beta")
	}
}

func TestBoundaryRejectsStateRootInsideCodeCheckout(t *testing.T) {
	dir := t.TempDir()
	os.Mkdir(filepath.Join(dir, ".git"), 0o755)
	writeFile(t, filepath.Join(dir, "plugroot.toml"), "\n[plugroot]\nstate_root = \"state\"\n")
	findings, err := boundaryFindings(mustContext(t, dir))
	if err != nil {
		t.Fatal(err)
	}
	for _, f := range findings {
		if f.severity == "error" && strings.Contains(f.message, "state root") {
			return
		}
	}
	t.Fatal(findings)
}

// --- interfaces.rs ---

func TestTUIRefreshReloadsManifestOverlay(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "plugroot.toml"), "\n[[service]]\nid = \"alpha\"\nname = \"Alpha\"\nkind = \"noop\"\n")
	app := newTUIApp(mustContext(t, dir))
	if len(app.rows) != 1 || app.rows[0].ID != "alpha" {
		t.Fatal(app.rows)
	}
	writeFile(t, filepath.Join(dir, "plugroot.local.toml"), "\n[[service]]\nid = \"beta\"\nname = \"Beta\"\nkind = \"noop\"\n")
	app.refresh()
	found := false
	for _, row := range app.rows {
		found = found || row.ID == "beta"
	}
	if !found || app.rows[app.selected].ID != "alpha" {
		t.Fatal(app.rows, app.selected)
	}
}

func TestTUIWindowMarksHorizontalOverflow(t *testing.T) {
	for _, c := range []struct {
		in            string
		width, offset int
		want          string
	}{{"abcdef", 4, 0, "abc>"}, {"abcdef", 4, 1, "<cd>"}, {"abc", 5, 0, "abc  "}} {
		if got := fitWindow(c.in, c.width, c.offset); got != c.want {
			t.Fatalf("%q: %q", c.in, got)
		}
	}
}

func testStatusRow(id, url string) StatusRow {
	return StatusRow{ID: id, Name: id, Plane: "private", Category: "app", Kind: "noop", State: "online",
		Detail: "running", URL: strp(url), Ports: []string{}, Controls: []string{}}
}

func TestTUIViewportKeepsSelectionVisibleAndClampsPan(t *testing.T) {
	app := &tuiApp{
		ctx: &Context{EnvValues: map[string]string{}},
		rows: []StatusRow{
			testStatusRow("alpha", "https://example.invalid/a"),
			testStatusRow("beta", "https://example.invalid/b"),
			testStatusRow("gamma", "https://example.invalid/very/long/service/link"),
			testStatusRow("delta", "https://example.invalid/d"),
		},
		selected: 3,
		scrollX:  math.MaxInt,
	}
	app.clampViewport(2, 24)
	if start, end := visibleRows(app, 2); start != 2 || end != 4 {
		t.Fatal(start, end)
	}
	if app.scrollX <= 0 || app.scrollX > max(tuiContentWidth(app)-24, 0) {
		t.Fatal(app.scrollX)
	}
}

func TestWebServiceExposesOneTapNameAndOpenLinks(t *testing.T) {
	html := renderWebService(testStatusRow("example", "https://example.invalid"))
	launch, details := strings.Index(html, `class="launch"`), strings.Index(html, `class="svc-more"`)
	if !strings.HasPrefix(html, `<article class="svc">`) || !strings.Contains(html, `class="svc-title svc-direct"`) ||
		launch < 0 || details < 0 || launch >= details || !strings.Contains(html, `aria-label="Open example"`) ||
		strings.Count(html, `href="https://example.invalid"`) != 2 {
		t.Fatal(html)
	}
}

func TestWebServiceDoesNotShowPlaneOrCategoryNavigation(t *testing.T) {
	html := renderWebService(testStatusRow("example", "https://example.invalid"))
	if strings.Contains(html, "private") || strings.Contains(html, "app") {
		t.Fatal(html)
	}
}

func TestWebServiceListsPutUnlinkedServicesAtTheBottom(t *testing.T) {
	linked := testStatusRow("linked", "https://example.invalid/linked")
	unlinked := testStatusRow("unlinked", "https://example.invalid/unlinked")
	unlinked.URL = nil
	html := renderWebServiceLists([]StatusRow{unlinked, linked})
	a, b, c := strings.Index(html, ">linked</h3>"), strings.Index(html, "No direct link"), strings.Index(html, ">unlinked</h3>")
	if a < 0 || b < 0 || c < 0 || !(a < b && b < c) || !strings.Contains(html, "Status-only services") {
		t.Fatal(html)
	}
}

func TestComponentStatusIsMinimalVersionedAndSafeWithoutAuth(t *testing.T) {
	req := webRequest{method: "GET", path: "/.well-known/linuxmice/component", headers: [][2]string{{"x-correlation-id", "pilot:request-42"}}}
	status := componentStatusJSON(req)
	get := func(o jsonObject, key string) any {
		for _, f := range o {
			if f.key == key {
				return f.value
			}
		}
		return nil
	}
	if get(status, "schema_version") != "linuxmice.response.v1" || get(status, "component") != "katteke.plugroot" ||
		get(status, "operation") != "status" || get(status, "correlation_id") != "pilot:request-42" ||
		get(get(status, "data").(jsonObject), "services") != nil {
		t.Fatal(prettyJSON(status))
	}
}

func TestWebBindsDefaultEnvListAndRepeatedFlags(t *testing.T) {
	check := func(env *string, args []string, want ...string) {
		t.Helper()
		got, err := webBinds(env, args)
		if err != nil || strings.Join(got, "|") != strings.Join(want, "|") {
			t.Fatal(got, err)
		}
	}
	check(nil, nil, "127.0.0.1:8786")
	check(strp("192.0.2.10:8088, 127.0.0.1:8786,192.0.2.10:8088"), nil, "192.0.2.10:8088", "127.0.0.1:8786")
	check(strp("ignored:1"), []string{"--bind", "10.0.0.1:1", "--bind=127.0.0.1:8786"}, "10.0.0.1:1", "127.0.0.1:8786")
	if _, err := webBinds(nil, []string{"--bind"}); err == nil {
		t.Fatal("--bind without a value")
	}
	if _, err := webBinds(strp(" , "), nil); err == nil {
		t.Fatal("empty list")
	}
}

func webTestCtx(stateRoot string, env map[string]string) *Context {
	return &Context{Root: stateRoot, Manifest: Manifest{Plugroot: &PlugrootConfig{StateRoot: strp(stateRoot)}}, EnvValues: env}
}

func requestWithHeader(name, value string) webRequest {
	return webRequest{method: "POST", path: "/api/action/alpha/restart", headers: [][2]string{{name, value}}}
}

func TestWebRequiresPasswordOffLoopback(t *testing.T) {
	cgnat := fmt.Sprintf("100.%d.0.1:8786", 64)
	for bind, want := range map[string]bool{
		"127.0.0.1:8786": true, "localhost:8786": true, "[::1]:8786": true,
		cgnat: false, "0.0.0.0:8786": false, "[::]:8786": false, "plugroot.internal:8786": false,
	} {
		if bindIsLoopback(bind) != want {
			t.Fatal(bind)
		}
	}
	loopback := []string{"127.0.0.1:8786"}
	mixed := []string{cgnat, "127.0.0.1:8786"}
	if requirePasswordOffLoopback(loopback, nil) != nil || requirePasswordOffLoopback(mixed, strp("pw")) != nil {
		t.Fatal("allowed cases refused")
	}
	err := requirePasswordOffLoopback(mixed, nil)
	if err == nil || !strings.Contains(err.Error(), "PLUGROOT_WEB_PASSWORD") || !strings.Contains(err.Error(), cgnat) {
		t.Fatal(err)
	}
}

func TestWebServesOverlayChangesAndKeepsLastGoodManifest(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, filepath.Join(dir, "plugroot.toml"), "\n[[service]]\nid = \"alpha\"\nname = \"Alpha\"\nkind = \"noop\"\n")
	live := &webContext{root: dir, binds: []string{"127.0.0.1:8786"}, ctx: mustContext(t, dir)}
	overlay := filepath.Join(dir, "plugroot.local.toml")
	writeFile(t, overlay, "\n[[service]]\nid = \"beta\"\nname = \"Beta\"\nkind = \"noop\"\nurl = \"https://beta.example\"\n")
	if s, ok := live.current().service("beta"); !ok || deref(s.URL, "") != "https://beta.example" {
		t.Fatal("a service added to the overlay is not served")
	}
	writeFile(t, overlay, "\n[[service]]\nid = \"beta\"\n")
	if _, ok := live.current().service("beta"); !ok || live.lastErr == "" {
		t.Fatal("an overlay that no longer loads replaced the last good manifest")
	}
	writeFile(t, overlay, "\n[[service]]\nid = \"gamma\"\nname = \"Gamma\"\nkind = \"noop\"\n")
	if _, ok := live.current().service("gamma"); !ok || live.lastErr != "" {
		t.Fatal("the fixed overlay is not served")
	}
}

func TestWebReloadNeverDropsThePasswordOffLoopback(t *testing.T) {
	code, state := t.TempDir(), t.TempDir()
	writeFile(t, filepath.Join(code, "plugroot.toml"), fmt.Sprintf("\n[plugroot]\nstate_root = %q\n", state))
	writeFile(t, filepath.Join(state, ".env"), "PLUGROOT_WEB_PASSWORD=hunter2\n")
	remote := &webContext{root: code, binds: []string{fmt.Sprintf("100.%d.0.1:8786", 64)}, ctx: mustContext(t, code)}
	loopback := &webContext{root: code, binds: []string{"127.0.0.1:8786"}, ctx: mustContext(t, code)}
	writeFile(t, filepath.Join(state, ".env"), "# the password line removed\n")
	if webPassword(remote.current()) == nil {
		t.Fatal("a reload left a non-loopback dashboard without a password")
	}
	if webPassword(loopback.current()) != nil {
		t.Fatal("a loopback dashboard did not take the new .env")
	}
}

func TestWebCtEqAndHMACVectors(t *testing.T) {
	if !ctEq(nil, nil) || !ctEq([]byte("abc"), []byte("abc")) || ctEq([]byte("abc"), []byte("abd")) ||
		ctEq([]byte("abc"), []byte("abcd")) || ctEq([]byte("abc"), nil) {
		t.Fatal("ctEq")
	}
	// RFC 4231 test case 2.
	if got := hex.EncodeToString(hmacSHA256([]byte("Jefe"), []byte("what do ya want for nothing?"))); got != "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843" {
		t.Fatal(got)
	}
	// RFC 4231 test case 6: a key longer than the block size.
	key := make([]byte, 131)
	for i := range key {
		key[i] = 0xaa
	}
	if got := hex.EncodeToString(hmacSHA256(key, []byte("Test Using Larger Than Block-Size Key - Hash Key First"))); got != "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54" {
		t.Fatal(got)
	}
}

func TestWebSessionCookieRoundTripUsesPersistedSecret(t *testing.T) {
	dir := t.TempDir()
	env := map[string]string{"PLUGROOT_WEB_PASSWORD": "hunter2", "PLUGROOT_WEB_USER": "rei"}
	ctx := webTestCtx(dir, env)
	cookie := webSessionCookie(ctx)
	value, _, _ := strings.Cut(cookie, ";")
	if !strings.HasPrefix(value, "plugroot_session=") || !webSessionValid(ctx, value) || !webAuthorized(ctx, requestWithHeader("cookie", value)) {
		t.Fatal(cookie)
	}
	keyPath := filepath.Join(dir, "web-session.key")
	secret := webSessionSecret(ctx)
	if string(secret) != string(webSessionSecret(ctx)) || string(secret) == string(make([]byte, 32)) {
		t.Fatal("secret not stable or zero")
	}
	text, _ := os.ReadFile(keyPath)
	if len(strings.TrimSpace(string(text))) != 64 || strings.Contains(string(text), "hunter2") {
		t.Fatal(string(text))
	}
	if info, _ := os.Stat(keyPath); info.Mode().Perm() != 0o600 {
		t.Fatal(info.Mode())
	}
	rotated := webTestCtx(dir, map[string]string{"PLUGROOT_WEB_PASSWORD": "other", "PLUGROOT_WEB_USER": "rei"})
	if !webSessionValid(rotated, value) {
		t.Fatal("the cookie depends on the password")
	}
	expires, signature, _ := strings.Cut(strings.TrimPrefix(value, "plugroot_session="), ".")
	flipped := "0" + signature[1:]
	if strings.HasPrefix(signature, "0") {
		flipped = "1" + signature[1:]
	}
	if webSessionValid(ctx, "plugroot_session="+expires+"."+flipped) || webSessionValid(ctx, "plugroot_session=1."+signature) ||
		webSessionValid(ctx, "plugroot_session=garbage") || webSessionValid(webTestCtx(t.TempDir(), env), value) {
		t.Fatal("tampered, expired or foreign cookie accepted")
	}
}

func TestWebBasicAuthComparesExactHeader(t *testing.T) {
	ctx := webTestCtx(t.TempDir(), map[string]string{"PLUGROOT_WEB_PASSWORD": "hunter2"})
	if !webAuthorized(ctx, requestWithHeader("authorization", "Basic cGx1Z3Jvb3Q6aHVudGVyMg==")) || // plugroot:hunter2
		webAuthorized(ctx, requestWithHeader("authorization", "Basic cGx1Z3Jvb3Q6aHVudGVyMw==")) || // plugroot:hunter3
		webAuthorized(ctx, requestWithHeader("authorization", "")) {
		t.Fatal("basic auth")
	}
}

func TestWebBodyCapRejectsLargeContentLengthBeforeReading(t *testing.T) {
	if declaredContentLength([]byte("POST / HTTP/1.1\r\nContent-Length: 12")) != 12 ||
		declaredContentLength([]byte("POST / HTTP/1.1\r\ncontent-length: x")) != 0 || maxHTTPBody != 65536 {
		t.Fatal("content length")
	}
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	client, err := net.Dial("tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer client.Close()
	server, err := listener.Accept()
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()
	readExact := func(n int) []byte {
		data := make([]byte, n)
		for got := 0; got < n; {
			m, err := server.Read(data[got:])
			if err != nil {
				t.Fatal(err)
			}
			got += m
		}
		return data
	}
	head := fmt.Sprintf("POST /login HTTP/1.1\r\nContent-Length: %d\r\n\r\n", maxHTTPBody+1)
	client.Write([]byte(head))
	data := readExact(len(head))
	if ok, err := readHTTPBody(server, &data); ok || err != nil || len(data) != len(head) {
		t.Fatal(ok, err, len(data))
	}
	head = "POST /login HTTP/1.1\r\nContent-Length: 5\r\n\r\n"
	client.Write([]byte(head))
	client.Write([]byte("a=b&c"))
	data = readExact(len(head))
	if ok, err := readHTTPBody(server, &data); !ok || err != nil || !strings.HasSuffix(string(data), "a=b&c") {
		t.Fatal(ok, err, string(data))
	}
}

// --- operations.rs ---

func TestEmergencyExportParsesSafetyOptions(t *testing.T) {
	options, ok, err := parseEmergencyExportOptions([]string{"--output=/mnt/usb", "--yes", "--dry-run", "--passphrase-file", "/tmp/passphrase"})
	if err != nil || !ok || options.outputDir != "/mnt/usb" || !options.yes || !options.dryRun ||
		deref(options.passphraseFile, "") != "/tmp/passphrase" || options.createOutputDir || options.ownerUser != nil {
		t.Fatal(options, ok, err)
	}
}

func TestSimpleBackupArgsUseWordsNotFlags(t *testing.T) {
	def := "/home/example/Documents/plugroot-emergency-exports"
	output, dryRun, ok, err := parseSimpleBackupArgs([]string{"check", "/media/example/USB"}, def)
	if err != nil || !ok || output != "/media/example/USB" || !dryRun {
		t.Fatal(output, dryRun, ok, err)
	}
	if _, _, _, err := parseSimpleBackupArgs([]string{"--output=/tmp"}, def); err == nil {
		t.Fatal("flag accepted")
	}
}

func TestApplyOptionsKeepRepoSyncExplicit(t *testing.T) {
	if o, err := parseApplyOptions([]string{"--dry-run"}); err != nil || o != (applyOptions{dryRun: true}) {
		t.Fatal(o, err)
	}
	if o, err := parseApplyOptions([]string{"--sync-repos"}); err != nil || o != (applyOptions{syncRepos: true}) {
		t.Fatal(o, err)
	}
	if _, err := parseApplyOptions([]string{"--pull"}); err == nil {
		t.Fatal("--pull accepted")
	}
}

func gitInit(t *testing.T, dir string) {
	t.Helper()
	cmd := exec.Command("git", "init", "--quiet")
	cmd.Dir = dir
	if err := cmd.Run(); err != nil {
		t.Fatal(err)
	}
}

func symlinkCtx(code, state string) *Context {
	return &Context{Root: code, Manifest: Manifest{Plugroot: &PlugrootConfig{StateRoot: strp(state)}}, EnvValues: map[string]string{}}
}

func TestManagedSymlinkProjectsIgnoredStateIntoGitCheckout(t *testing.T) {
	code, state := t.TempDir(), t.TempDir()
	checkout := filepath.Join(code, "example-app")
	os.Mkdir(checkout, 0o755)
	gitInit(t, checkout)
	writeFile(t, filepath.Join(checkout, ".gitignore"), ".env*\n")
	workspace := filepath.Join(state, "workspaces/example-app")
	os.MkdirAll(workspace, 0o755)
	source := filepath.Join(workspace, ".env")
	writeFile(t, source, "private fixture\n")
	target := filepath.Join(checkout, ".env")
	ctx := symlinkCtx(code, state)
	link := ManagedSymlink{Source: source, Target: target}
	if err := applySymlink(ctx, link); err != nil {
		t.Fatal(err)
	}
	if err := applySymlink(ctx, link); err != nil {
		t.Fatal("a second apply must be a no-op:", err)
	}
	info, err := os.Lstat(target)
	if err != nil || info.Mode()&os.ModeSymlink == 0 {
		t.Fatal("not a symlink")
	}
	a, _ := filepath.EvalSymlinks(target)
	b, _ := filepath.EvalSymlinks(source)
	if a != b {
		t.Fatal(a, b)
	}
	existing := filepath.Join(checkout, ".env.local")
	writeFile(t, existing, "keep this file\n")
	err = applySymlink(ctx, ManagedSymlink{Source: source, Target: existing})
	if errKindOf(err) != errAlreadyExists {
		t.Fatal(err)
	}
	if text, _ := os.ReadFile(existing); string(text) != "keep this file\n" {
		t.Fatal(string(text))
	}
}

func TestManagedSymlinkRefusesUnignoredOrExternalPrivateState(t *testing.T) {
	code, state := t.TempDir(), t.TempDir()
	checkout := filepath.Join(code, "example-app")
	os.Mkdir(checkout, 0o755)
	gitInit(t, checkout)
	source := filepath.Join(state, "secret.env")
	writeFile(t, source, "private fixture\n")
	ctx := symlinkCtx(code, state)
	if err := applySymlink(ctx, ManagedSymlink{Source: source, Target: filepath.Join(checkout, "secret.env")}); errKindOf(err) != errInvalidInput {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(checkout, ".gitignore"), ".env\n")
	external := filepath.Join(code, "external.env")
	writeFile(t, external, "private fixture\n")
	if err := applySymlink(ctx, ManagedSymlink{Source: external, Target: filepath.Join(checkout, ".env")}); errKindOf(err) != errInvalidInput {
		t.Fatal(err)
	}
}

func TestComposeUsesProcessEnvInsteadOfRawEnvFile(t *testing.T) {
	ctx := &Context{Root: "/srv/plugroot", EnvValues: map[string]string{"PLUGDECK_PASSWORD_HASH": "$argon2id$v=19$m=1"}}
	s := Service{ID: "example", Name: "Example", Kind: "compose", Path: strp("/srv/plugroot-state/.plugroot/local/compose/example")}
	args, ok := composeArgs(ctx, s)
	if !ok {
		t.Fatal("no args")
	}
	for _, arg := range args {
		if arg == "--env-file" {
			t.Fatal(args)
		}
	}
	if composeEnvValues(ctx, s)["PLUGDECK_PASSWORD_HASH"] != "$argon2id$v=19$m=1" {
		t.Fatal("env")
	}
	want := "compose --project-directory /srv/plugroot-state/.plugroot/local/compose/example -f /srv/plugroot-state/.plugroot/local/compose/example/compose.yaml"
	if strings.Join(args, " ") != want {
		t.Fatal(args)
	}
}

func TestRestoreClassifiesSupportedArchives(t *testing.T) {
	if k, ok := classifyRestoreArchive("plugroot-emergency-host.tar.zst.gpg"); !ok || k != restoreEncrypted {
		t.Fatal(k)
	}
	if k, ok := classifyRestoreArchive("plugroot-plain-emergency-host.tar.zst"); !ok || k != restorePlain {
		t.Fatal(k)
	}
	if _, ok := classifyRestoreArchive("notes.txt"); ok {
		t.Fatal("notes.txt")
	}
}

func TestRestoreFindsNewestArchiveInDirectory(t *testing.T) {
	dir := t.TempDir()
	old, newer := filepath.Join(dir, "plugroot-emergency-old.tar.zst.gpg"), filepath.Join(dir, "plugroot-emergency-new.tar.zst.gpg")
	writeFile(t, old, "old")
	time.Sleep(5 * time.Millisecond)
	writeFile(t, newer, "new")
	if got, err := findRestoreArchive(dir); err != nil || got != newer {
		t.Fatal(got, err)
	}
}

func TestRestorePrefersEncryptedArchiveOverPlain(t *testing.T) {
	dir := t.TempDir()
	encrypted, plain := filepath.Join(dir, "plugroot-emergency-old.tar.zst.gpg"), filepath.Join(dir, "plugroot-plain-emergency-new.tar.zst")
	writeFile(t, encrypted, "encrypted")
	time.Sleep(5 * time.Millisecond)
	writeFile(t, plain, "plain")
	if got, err := findRestoreArchive(dir); err != nil || got != encrypted {
		t.Fatal(got, err)
	}
}

func TestEmergencyExportRejectsOutputInsideCodeOrStateRoot(t *testing.T) {
	code, state, outside := t.TempDir(), t.TempDir(), t.TempDir()
	codeOutput, stateOutput := filepath.Join(code, "exports"), filepath.Join(state, "exports")
	os.Mkdir(codeOutput, 0o755)
	os.Mkdir(stateOutput, 0o755)
	ctx := symlinkCtx(code, state)
	if validateEmergencyOutputBoundary(ctx, codeOutput) == nil || validateEmergencyOutputBoundary(ctx, stateOutput) == nil ||
		validateEmergencyOutputBoundary(ctx, outside) != nil || validateEmergencyOutputDir(outside) != nil {
		t.Fatal("output boundary")
	}
}

func TestEmergencyExportSanitizesArchiveNameParts(t *testing.T) {
	if sanitizeFilenameComponent("mint laptop/desk") != "mint-laptop-desk" || sanitizeFilenameComponent("!!!") != "plugroot-host" {
		t.Fatal("sanitize")
	}
}

func TestEmergencyTarPathsDropLeadingRoot(t *testing.T) {
	if got, ok := tarRelativePath("/srv/plugroot/target"); !ok || got != "srv/plugroot/target" {
		t.Fatal(got)
	}
}

func TestEmergencyTarExcludesStateRepoBuildCaches(t *testing.T) {
	ctx := &Context{Root: "/srv/plugroot", Manifest: Manifest{Plugroot: &PlugrootConfig{StateRoot: strp("/srv/plugroot-state")}}}
	excludes := strings.Join(emergencyTarExcludes(ctx), "\n") + "\n"
	for _, want := range []string{"srv/plugroot/target", "srv/plugroot-state/repos/*/target", "srv/plugroot-state/repos/*/.cache", "srv/plugroot-state/repos/*/node_modules"} {
		if !strings.Contains(excludes, want+"\n") {
			t.Fatal(excludes)
		}
	}
}

func TestGeneratesUserUnit(t *testing.T) {
	command := []string{"/usr/bin/python3", "server.py"}
	s := Service{ID: "example-app", Name: "Example App", Kind: "user-systemd", Unit: strp("example-app.service"),
		User: strp("plugroot"), WorkingDir: strp("/tmp/example-app"), Command: &command, Env: []string{"EXAMPLE_LOG_REQUESTS=0"}}
	unit, ok, err := generateUnit(&Context{}, s)
	if err != nil || !ok || !strings.Contains(unit, "ExecStart=/usr/bin/python3 server.py") || !strings.Contains(unit, "WantedBy=default.target") {
		t.Fatal(unit, ok, err)
	}
}

func TestReleaseInstallFlipsCurrentAndRollbackReturns(t *testing.T) {
	state := t.TempDir()
	dir := filepath.Join(state, "releases", "demo")
	binary := filepath.Join(state, "demo-build")
	writeFile(t, binary, "#!/bin/sh\necho demo\n")
	first, err := installRelease(dir, "demo", binary, "20260101000000")
	if err != nil || first != (releaseChange{current: "20260101000000"}) {
		t.Fatal(first, err)
	}
	installed := filepath.Join(dir, "current", "demo")
	if a, _ := os.ReadFile(installed); string(a) != "#!/bin/sh\necho demo\n" {
		t.Fatal(string(a))
	}
	if info, _ := os.Stat(installed); info.Mode().Perm() != 0o755 {
		t.Fatal(info.Mode())
	}
	second, err := installRelease(dir, "demo", binary, "20260102000000")
	if err != nil || second.previous != "20260101000000" {
		t.Fatal(second, err)
	}
	if target, _ := os.Readlink(filepath.Join(dir, "current")); target != "20260102000000" {
		t.Fatal(target)
	}
	if !exists(filepath.Join(dir, "20260101000000", "demo")) {
		t.Fatal("old build kept")
	}
	listing, err := listReleases(dir)
	if err != nil || listing.current != "20260102000000" || strings.Join(listing.releases, ",") != "20260101000000,20260102000000" {
		t.Fatal(listing, err)
	}
	back, err := rollbackRelease(dir)
	if err != nil || back.current != "20260101000000" || back.previous != "20260102000000" {
		t.Fatal(back, err)
	}
	if _, err := rollbackRelease(dir); err == nil {
		t.Fatal("nothing older than the first build")
	}
	if _, err := installRelease(dir, "demo", binary, "20260102000000"); err == nil {
		t.Fatal("same stamp twice")
	}
	if uniqueStamp(dir, "20260102000000") != "20260102000000-2" || uniqueStamp(dir, "20260109000000") != "20260109000000" {
		t.Fatal("unique stamp")
	}
	if _, err := installRelease(dir, "demo", binary, "current"); err == nil {
		t.Fatal("current as a stamp")
	}
	if _, err := installRelease(dir, "demo", dir, "20260103000000"); err == nil {
		t.Fatal("a directory as the binary")
	}
	if !exists(filepath.Join(dir, "current", "demo")) {
		t.Fatal("current lost")
	}
}

func TestReleaseStampIsUTCCivilTime(t *testing.T) {
	for secs, want := range map[int64]string{0: "19700101000000", 1_700_000_000: "20231114221320", 951_782_400: "20000229000000"} {
		if got := stampFromUnix(secs); got != want {
			t.Fatal(secs, got)
		}
	}
	now := releaseStamp()
	if len(now) != 14 || strings.Trim(now, "0123456789") != "" {
		t.Fatal(now)
	}
}

// --- security.rs ---

func TestAuditSecretAssignmentAllowsPlaceholders(t *testing.T) {
	if suspiciousSecretAssignment("TOKEN=") || suspiciousSecretAssignment("TOKEN=<value>") ||
		suspiciousSecretAssignment("TOKEN=${VALUE}") || !suspiciousSecretAssignment("TOKEN=abc123") {
		t.Fatal("placeholders")
	}
}

func TestAuditDetectsCGNATPrivateAddress(t *testing.T) {
	if !containsTailscaleIPv4(fmt.Sprintf("bind = 100.%d.0.1", 64)) || containsTailscaleIPv4("bind = 127.0.0.1") {
		t.Fatal("cgnat")
	}
}

func TestAuditTokenMarkerRequiresTokenShapedSuffix(t *testing.T) {
	marker := "s" + "k-"
	image := "image: rust" + "desk/rustde" + "sk" + "-server:latest"
	if containsTokenMarker(image, marker) || !containsTokenMarker("OPENAI_API_KEY="+marker+"abcdefghijklmnop", marker) {
		t.Fatal("token marker")
	}
}

func TestAuditRejectsPrivatePaths(t *testing.T) {
	if _, ok := auditPath("docs/private/notes.md"); !ok {
		t.Fatal("docs/private")
	}
	if _, ok := auditPath("service/data/state.db"); !ok {
		t.Fatal("data")
	}
	if _, ok := auditPath(".env.example"); ok {
		t.Fatal(".env.example")
	}
}

func TestAuditDenylistIdentifiersRequireBoundaries(t *testing.T) {
	if !containsDenylistTerm("workspace/sam/project", "sam") || !containsDenylistTerm("host=SAM", "sam") ||
		containsDenylistTerm("sample configuration", "sam") ||
		!containsDenylistTerm("origin=https://private.example.test/repo", "private.example.test") {
		t.Fatal("denylist")
	}
}

func TestDoctorParsesManifestTCPPortRanges(t *testing.T) {
	ports := map[uint16]bool{}
	addTCPPortsFromDescriptor("21115-21119/tcp tailscale", ports)
	addTCPPortsFromDescriptor("21120/udp tailscale", ports)
	if !ports[21115] || !ports[21119] || ports[21120] {
		t.Fatal(ports)
	}
}

func TestDoctorFlagsOnlyBroadUFWAllows(t *testing.T) {
	rules := broadUFWAllowRules("\nStatus: active\n22/tcp on tailscale0       ALLOW IN    Anywhere\n41641/udp                  ALLOW IN    Anywhere\n8080/tcp                   ALLOW IN    Anywhere\n")
	if len(rules) != 1 || rules[0] != "8080/tcp                   ALLOW IN    Anywhere" {
		t.Fatal(rules)
	}
}

func TestDoctorFlagsUnexpectedWildcardTCPListeners(t *testing.T) {
	listeners := unexpectedWildcardTCPListeners(fmt.Sprintf("\ntcp LISTEN 0 4096 0.0.0.0:22 0.0.0.0:*\ntcp LISTEN 0 4096 100.%d.0.1:8088 0.0.0.0:*\ntcp LISTEN 0 4096 *:7345 *:*\n", 64), map[uint16]bool{22: true})
	if len(listeners) != 1 || !strings.Contains(listeners[0], "*:7345") {
		t.Fatal(listeners)
	}
}

// --- restore/plan.rs ---

func testPlan(code, state string) *restorePlan {
	return &restorePlan{archive: "/tmp/example.tar.zst", kind: restorePlain, codeRoot: code, stateRoot: state}
}

func TestDefaultRestoreTargetsFollowTheSelectedCodeRoot(t *testing.T) {
	code, state, err := restoreTargetsFromManifest("/srv/example", "")
	if err != nil || code != "/srv/example" || state != "/srv/example-state" {
		t.Fatal(code, state, err)
	}
}

func TestRestoreTargetsMustBeDistinctAbsoluteRoots(t *testing.T) {
	if validateRestoreTargets(testPlan("/srv/example", "/srv/example-state")) != nil ||
		validateRestoreTargets(testPlan("example", "/srv/example-state")) == nil ||
		validateRestoreTargets(testPlan("/srv/example", "/srv/example/data")) == nil {
		t.Fatal("restore targets")
	}
}

func TestArchiveMembersFollowTheRestoreTargets(t *testing.T) {
	roots, err := restoreArchiveRoots(testPlan("/srv/example", "/srv/example-state"))
	if err != nil || roots != [2]string{"srv/example", "srv/example-state"} {
		t.Fatal(roots, err)
	}
}
