package main

// Tests for what the Rust version took from crates: the TOML reader and
// the manifest decoding (toml, serde), Rust's path rules (std::path), the
// pretty JSON (serde_json) and lossy UTF-8 (std).

import (
	"strings"
	"testing"
)

func TestTOMLSubsetAndManifestDecoding(t *testing.T) {
	table, err := parseTOML("[host]\nname = \"h\" # comment\n\n[[service]]\nid = \"a\"\nname = \"A\"\nkind = \"port\"\nport = 8_080\ncontrols = [\n  \"start\",\n  \"stop\", # trailing comma\n]\noptional = true\nunknown_key = 'ignored'\ncommand = []\n")
	if err != nil {
		t.Fatal(err)
	}
	m, err := decodeManifest(table)
	if err != nil {
		t.Fatal(err)
	}
	s := m.Service[0]
	if deref(m.Host.Name, "") != "h" || *s.Port != 8080 || len(s.Controls) != 2 || !*s.Optional || s.Command == nil || len(*s.Command) != 0 {
		t.Fatalf("%+v", s)
	}
	for text, want := range map[string]string{
		"[[service]]\nid = \"a\"\nkind = \"x\"\n":                              "missing field `name`",
		"[[service]]\nid = \"a\"\nname = \"A\"\nkind = \"x\"\nport = 70000\n":  "service[0].port: expected an integer from 0 to 65535",
		"[[service]]\nid = \"a\"\nname = \"A\"\nkind = \"x\"\nport = \"80\"\n": "service[0].port",
		"plugroot = \"x\"\n": "plugroot: expected a table",
		"[[service]]\nid = 1\nname = \"A\"\nkind = \"x\"\n": "service[0].id: expected a string",
		"[[repo]]\nid = \"r\"\nurl = \"u\"\n":               "missing field `path`",
	} {
		table, err := parseTOML(text)
		if err == nil {
			_, err = decodeManifest(table)
		}
		if err == nil || !strings.Contains(err.Error(), want) {
			t.Fatalf("%q: %v", text, err)
		}
	}
	for _, bad := range []string{"a = 1\na = 2\n", "[t]\n[t]\n", "a = 1.5\n", "a = {b = 1}\n", "a.b = 1\n", "a = \"\"\"x\"\"\"\n", "a = 2026-11-08\n"} {
		if _, err := parseTOML(bad); err == nil {
			t.Fatalf("%q accepted", bad)
		}
	}
}

func TestTheRepositoryManifestsParse(t *testing.T) {
	for _, path := range []string{"plugroot.toml", "plugroot.local.example.toml", "linuxmice-component.toml"} {
		text, err := readText(path)
		if err != nil {
			t.Fatal(err)
		}
		table, err := parseTOML(expandVars(text, map[string]string{}))
		if err != nil {
			t.Fatal(path, err)
		}
		if path != "linuxmice-component.toml" {
			if _, err := decodeManifest(table); err != nil {
				t.Fatal(path, err)
			}
		}
	}
}

func TestOverlaysReplaceByIDAndAppend(t *testing.T) {
	base := &Manifest{Service: []Service{{ID: "a", Name: "A"}, {ID: "b", Name: "B"}}, Host: &HostConfig{Name: strp("base")}}
	overlay := &Manifest{Service: []Service{{ID: "b", Name: "B2"}, {ID: "c", Name: "C"}}}
	m := mergeManifest(base, overlay)
	if len(m.Service) != 3 || m.Service[1].Name != "B2" || m.Service[2].ID != "c" || deref(m.Host.Name, "") != "base" {
		t.Fatalf("%+v", m.Service)
	}
}

func TestEnvFiles(t *testing.T) {
	dir := t.TempDir()
	writeFile(t, dir+"/.env", "# note\nA=1\n B = \"two\" \nC='three'\nnot a pair\nD=x=y\n")
	got := loadEnvFile(dir + "/.env")
	if got["A"] != "1" || got["B"] != "two" || got["C"] != "three" || got["D"] != "x=y" || len(got) != 4 {
		t.Fatal(got)
	}
	if len(loadEnvFile(dir+"/missing")) != 0 {
		t.Fatal("missing file")
	}
}

func TestPathsFollowRust(t *testing.T) {
	for _, c := range []struct{ base, rest, want string }{
		{"/a", "b", "/a/b"}, {"/a/", "b", "/a/b"}, {"/a", "/b", "/b"}, {"", "b", "b"}, {".", "b", "./b"},
	} {
		if got := joinPath(c.base, c.rest); got != c.want {
			t.Fatal(c, got)
		}
	}
	for in, want := range map[string]string{"/a/./b/../c": "/a/c", "a/../../b": "b", "/..": "/", "./a/": "a", "": "", "/": "/"} {
		if got := cleanPath(in); got != want {
			t.Fatal(in, got)
		}
	}
	if !sameOrInside("/srv/x", "/srv/x/y") || sameOrInside("/srv/x", "/srv/x-state") || !sameOrInside("/srv/x/", "/srv/x") {
		t.Fatal("sameOrInside")
	}
	for in, want := range map[string]string{"/a/b": "/a", "/a": "/", "a": "", "a/b/": "a"} {
		if got, ok := parentPath(in); !ok || got != want {
			t.Fatal(in, got)
		}
	}
	if _, ok := parentPath("/"); ok {
		t.Fatal("root has no parent")
	}
	for in, want := range map[string]string{"a.db": "db", "a.b.db": "db", "a.": ""} {
		if got, ok := extension(in); !ok || got != want {
			t.Fatal(in, got)
		}
	}
	for _, none := range []string{".db", "db", ".."} {
		if _, ok := extension(none); ok {
			t.Fatal(none)
		}
	}
	if defaultStateRoot("/") != "plugroot-state" || defaultStateRoot("/srv/code/") != "/srv/code-state" {
		t.Fatal(defaultStateRoot("/"))
	}
}

func TestPrettyJSONMatchesSerde(t *testing.T) {
	got := prettyJSON([]any{jsonObject{{"a", "x<&>\n\x01 "}, {"b", []string{}}, {"c", nil}, {"d", jsonObject{}}, {"e", true}}})
	want := "[\n  {\n    \"a\": \"x<&>\\n\\u0001 \",\n    \"b\": [],\n    \"c\": null,\n    \"d\": {},\n    \"e\": true\n  }\n]"
	if got != want {
		t.Fatalf("%q", got)
	}
	if prettyJSON([]any{}) != "[]" {
		t.Fatal("empty list")
	}
}

func TestLossyUTF8MatchesRust(t *testing.T) {
	for in, want := range map[string]string{
		"ok":               "ok",
		"a\xffb":           "a�b",
		"\xe2\x82A":        "�A", // one replacement for a cut-off sequence
		"\xe2\x82\xe2\x82": "��",
		"\xf0\x9f\x98":     "�",
		"\xed\xa0\x80":     "���", // a surrogate is three
	} {
		if got := lossyUTF8([]byte(in)); got != want {
			t.Fatalf("%q: %q", in, got)
		}
	}
}

func TestRustLines(t *testing.T) {
	if strings.Join(rustLines("a\r\nb\n\nc\n"), "|") != "a|b||c" || len(rustLines("")) != 0 || len(rustLines("\n")) != 1 {
		t.Fatal(rustLines("a\r\nb\n\nc\n"))
	}
}

func TestWebRequestParsing(t *testing.T) {
	req, ok := parseWebRequest("POST /api/action/a/start?x=1 HTTP/1.1\r\nHost: h\r\nCookie:  plugroot_session=1.2 \r\n\r\nbody")
	if !ok || req.method != "POST" || req.path != "/api/action/a/start" || req.body != "body" || req.headers[1] != [2]string{"cookie", "plugroot_session=1.2"} {
		t.Fatalf("%+v", req)
	}
	if _, ok := parseWebRequest(""); ok {
		t.Fatal("empty request")
	}
	if _, ok := parseWebRequest("GET\r\n\r\n"); ok {
		t.Fatal("no path")
	}
	fields := parseFormBody("user=a+b&password=%41%42&bad=%zz&user=last")
	if fields["user"] != "last" || fields["password"] != "AB" {
		t.Fatal(fields)
	}
	if _, ok := fields["bad"]; ok {
		t.Fatal("undecodable pair kept")
	}
	if declaredContentLength([]byte("POST / HTTP/1.1\r\nContent-Length: x\r\nContent-Length: +7")) != 7 {
		t.Fatal("the first parsable length wins")
	}
}
