package main

// plugroot web: a small dashboard on raw TCP, written byte for byte as the
// Rust version answered: a status page and JSON, start/stop/restart/logs
// actions, a login with an HMAC-signed session cookie, Basic auth, and an
// unauthenticated component status for LinuxMice.

import (
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"
)

type webRequest struct {
	method  string
	path    string
	headers [][2]string // lower-case name, trimmed value
	body    string
}

// maxHTTPBody is the largest request body read, checked before
// authentication.
const maxHTTPBody = 64 * 1024

func cmdWeb(ctx *Context, args []string) (int, error) {
	envBind, given := ctx.EnvValues["PLUGROOT_WEB_BIND"]
	if !given {
		envBind, given = os.LookupEnv("PLUGROOT_WEB_BIND")
	}
	var envBindPtr *string
	if given {
		envBindPtr = &envBind
	}
	binds, err := webBinds(envBindPtr, args)
	if err != nil {
		return 0, err
	}
	if err := requirePasswordOffLoopback(binds, webPassword(ctx)); err != nil {
		return 0, err
	}
	var listeners []net.Listener
	for _, bind := range binds {
		listener, err := listen(bind)
		if err != nil {
			for _, l := range listeners {
				l.Close()
			}
			return 0, err
		}
		listeners = append(listeners, listener)
		fmt.Printf("Plugroot web listening on http://%s\n", bind)
	}
	var wg sync.WaitGroup
	for _, listener := range listeners {
		wg.Add(1)
		go func(l net.Listener) {
			defer wg.Done()
			serveWebListener(ctx, l)
		}(listener)
	}
	wg.Wait()
	return 0, nil
}

// listen binds one address as Rust's TcpListener did: an IPv4 address gets
// an IPv4 socket, an IPv6 one an IPv6 socket with the system's dual-stack
// default, and a name the first of its addresses.
func listen(bind string) (net.Listener, error) {
	host, _, err := net.SplitHostPort(bind)
	if err != nil {
		return nil, errors.New("invalid socket address")
	}
	network := "tcp"
	if ip := net.ParseIP(host); ip != nil && ip.To4() != nil && !strings.Contains(host, ":") {
		network = "tcp4"
	}
	listener, err := net.Listen(network, bind)
	if err != nil {
		return nil, ioError(err)
	}
	return listener, nil
}

// webBinds resolves the listen addresses. Repeated --bind flags win over
// PLUGROOT_WEB_BIND; the environment value may list several comma-separated
// addresses, for example a private carrier address plus 127.0.0.1:8786 for
// the loopback LinuxMice component probe.
func webBinds(envBind *string, args []string) ([]string, error) {
	var flagBinds []string
	for i := 0; i < len(args); i++ {
		switch arg := args[i]; {
		case arg == "--bind":
			if i+1 >= len(args) {
				return nil, errors.New("--bind requires addr:port")
			}
			flagBinds = append(flagBinds, args[i+1])
			i++
		case strings.HasPrefix(arg, "--bind="):
			flagBinds = append(flagBinds, trimAllPrefix(arg, "--bind="))
		}
	}
	raw := flagBinds
	if len(raw) == 0 {
		value := "127.0.0.1:8786"
		if envBind != nil {
			value = *envBind
		}
		raw = []string{value}
	}
	var binds []string
	for _, entry := range raw {
		for _, part := range strings.Split(entry, ",") {
			part = strings.TrimFunc(part, isRustWhitespace)
			if part == "" {
				continue
			}
			duplicate := false
			for _, b := range binds {
				if b == part {
					duplicate = true
				}
			}
			if !duplicate {
				binds = append(binds, part)
			}
		}
	}
	if len(binds) == 0 {
		return nil, errors.New("no web bind address configured")
	}
	return binds, nil
}

// requirePasswordOffLoopback refuses to serve unauthenticated on anything
// but loopback: without PLUGROOT_WEB_PASSWORD every bind must be loopback.
func requirePasswordOffLoopback(binds []string, password *string) error {
	if password != nil {
		return nil
	}
	var remote []string
	for _, bind := range binds {
		if !bindIsLoopback(bind) {
			remote = append(remote, bind)
		}
	}
	if len(remote) == 0 {
		return nil
	}
	return fmt.Errorf("refusing to serve the web dashboard without a password on %s: set PLUGROOT_WEB_PASSWORD (and optionally PLUGROOT_WEB_USER), or bind only loopback addresses via PLUGROOT_WEB_BIND / --bind", strings.Join(remote, ", "))
}

// bindIsLoopback: host:port with a loopback host. Names other than
// localhost are not resolved and count as remote.
func bindIsLoopback(bind string) bool {
	host := bind
	if i := strings.LastIndexByte(bind, ':'); i >= 0 {
		host = bind[:i]
	}
	host = strings.TrimRight(strings.TrimLeft(host, "["), "]")
	if strings.EqualFold(host, "localhost") && isASCII(host) {
		return true
	}
	ip := parseRustIP(host)
	if ip == nil {
		return false
	}
	if strings.Contains(host, ":") {
		// Rust's Ipv6Addr::is_loopback is ::1 only, not ::ffff:127.0.0.1.
		return ip.Equal(net.IPv6loopback)
	}
	return ip.IsLoopback()
}

func isASCII(s string) bool {
	for i := 0; i < len(s); i++ {
		if s[i] >= 0x80 {
			return false
		}
	}
	return true
}

// parseRustIP accepts what Rust's IpAddr parsing accepts: dotted IPv4
// without leading zeros, or an IPv6 address without a zone.
func parseRustIP(host string) net.IP {
	if strings.Contains(host, "%") {
		return nil
	}
	ip := net.ParseIP(host)
	if ip == nil {
		return nil
	}
	if !strings.Contains(host, ":") {
		for _, part := range strings.Split(host, ".") {
			if len(part) > 1 && part[0] == '0' {
				return nil
			}
		}
	}
	return ip
}

func serveWebListener(ctx *Context, listener net.Listener) {
	for {
		conn, err := listener.Accept()
		if err != nil {
			fmt.Fprintf(os.Stderr, "connection error: %s\n", ioError(err))
			if errors.Is(err, net.ErrClosed) {
				return
			}
			continue
		}
		go func() {
			defer conn.Close()
			handleHTTP(ctx, conn)
		}()
	}
}

func handleHTTP(ctx *Context, conn net.Conn) {
	buf := make([]byte, 8192)
	n, err := conn.Read(buf)
	if err != nil && n == 0 && !errors.Is(err, io.EOF) {
		return
	}
	data := append([]byte{}, buf[:n]...)
	ok, err := readHTTPBody(conn, &data)
	if err != nil {
		return
	}
	if !ok {
		httpResponse(conn, 413, "text/plain", []byte("request body too large"))
		return
	}
	req, ok := parseWebRequest(lossyUTF8(data))
	if !ok {
		return
	}
	method, path := req.method, req.path
	switch {
	case method == "GET" && path == "/login":
		httpResponse(conn, 200, "text/html; charset=utf-8", []byte(renderWebLogin(ctx, "")))
		return
	case method == "POST" && path == "/login":
		handleWebLogin(ctx, conn, req)
		return
	case method == "POST" && path == "/logout":
		httpRedirectWithCookie(conn, "/login", "plugroot_session=; Max-Age=0; Path=/; HttpOnly; SameSite=Lax")
		return
	case method == "GET" && path == "/.well-known/linuxmice/component":
		httpResponse(conn, 200, "application/json", []byte(prettyJSON(componentStatusJSON(req))))
		return
	}
	if !webAuthorized(ctx, req) {
		if method == "GET" && path == "/" {
			httpRedirect(conn, "/login")
		} else {
			httpUnauthorized(conn)
		}
		return
	}
	switch {
	case method == "GET" && path == "/":
		httpResponse(conn, 200, "text/html; charset=utf-8", []byte(renderWeb(ctx)))
	case method == "GET" && path == "/api/status":
		httpResponse(conn, 200, "application/json", []byte(statusRowsJSON(statusRows(ctx))))
	case method == "POST" && strings.HasPrefix(path, "/api/action/"):
		segments := strings.Split(trimAllPrefix(path, "/api/action/"), "/")
		if len(segments) != 2 {
			httpResponse(conn, 400, "text/plain", []byte("bad action path"))
			return
		}
		id, action := segments[0], segments[1]
		s, ok := ctx.service(id)
		if !ok {
			httpResponse(conn, 404, "text/plain", []byte("unknown service"))
			return
		}
		if !hasControl(s, action) {
			httpResponse(conn, 403, "text/plain", []byte("action not allowed"))
			return
		}
		var out CmdOutput
		if action == "logs" {
			out = logsFor(ctx, s)
		} else {
			out = runFixedAction(ctx, s, action)
		}
		code := 200
		if out.Code != 0 {
			code = 500
		}
		httpResponse(conn, code, "text/plain", []byte(out.Text))
	default:
		httpResponse(conn, 404, "text/plain", []byte("not found"))
	}
}

func componentStatusJSON(req webRequest) jsonObject {
	return jsonObject{
		{"component", "katteke.plugroot"},
		{"correlation_id", componentCorrelationID(req)},
		{"data", jsonObject{
			{"capabilities", []string{"selfhost.manifest", "selfhost.lifecycle", "selfhost.backup", "selfhost.restore"}},
			{"standalone", true},
			{"status", "ready"},
			{"version", version},
		}},
		{"error", nil},
		{"operation", "status"},
		{"outcome", "success"},
		{"schema_version", "linuxmice.response.v1"},
		{"warnings", []string{}},
	}
}

func componentCorrelationID(req webRequest) string {
	for _, h := range req.headers {
		if h[0] != "x-correlation-id" {
			continue
		}
		value := h[1]
		valid := value != "" && len(value) <= 128
		for i := 0; valid && i < len(value); i++ {
			valid = denylistIdentifierByte(value[i]) || strings.IndexByte(".:", value[i]) >= 0
		}
		if valid {
			return value
		}
		break
	}
	return fmt.Sprintf("plugroot-%d-%d", os.Getpid(), time.Now().UnixNano())
}

// readHTTPBody reads the rest of the body Content-Length announces; false,
// without reading more, when it is larger than maxHTTPBody.
func readHTTPBody(conn net.Conn, data *[]byte) (bool, error) {
	headerEnd := findHeaderEnd(*data)
	if headerEnd < 0 {
		return true, nil
	}
	length := declaredContentLength((*data)[:headerEnd])
	if length > maxHTTPBody {
		return false, nil
	}
	needed := headerEnd + 4 + length
	for len(*data) < needed {
		buf := make([]byte, 4096)
		n, err := conn.Read(buf)
		if n == 0 {
			if err != nil && !errors.Is(err, io.EOF) {
				return false, err
			}
			break
		}
		*data = append(*data, buf[:n]...)
	}
	return true, nil
}

func declaredContentLength(head []byte) int {
	for _, line := range rustLines(lossyUTF8(head)) {
		name, value, ok := strings.Cut(line, ":")
		if !ok || !strings.EqualFold(name, "content-length") || !isASCII(name) {
			continue
		}
		if n, ok := parseUnsigned(strings.TrimFunc(value, isRustWhitespace), 64); ok && n <= uint64(^uint(0)>>1) {
			return int(n)
		}
	}
	return 0
}

func findHeaderEnd(data []byte) int {
	for i := 0; i+4 <= len(data); i++ {
		if data[i] == '\r' && data[i+1] == '\n' && data[i+2] == '\r' && data[i+3] == '\n' {
			return i
		}
	}
	return -1
}

func parseWebRequest(request string) (webRequest, bool) {
	head, body, found := strings.Cut(request, "\r\n\r\n")
	if !found {
		head, body = request, ""
	}
	lines := rustLines(head)
	if len(lines) == 0 {
		return webRequest{}, false
	}
	parts := strings.FieldsFunc(lines[0], isRustWhitespace)
	if len(parts) < 2 {
		return webRequest{}, false
	}
	path, _, _ := strings.Cut(parts[1], "?")
	req := webRequest{method: parts[0], path: path, body: body}
	for _, line := range lines[1:] {
		if name, value, ok := strings.Cut(line, ":"); ok {
			req.headers = append(req.headers, [2]string{asciiLower(strings.TrimFunc(name, isRustWhitespace)), strings.TrimFunc(value, isRustWhitespace)})
		}
	}
	return req, true
}

func webUser(ctx *Context) string {
	if user := ctx.EnvValues["PLUGROOT_WEB_USER"]; user != "" {
		return user
	}
	return "plugroot"
}

// webPassword is the dashboard password, nil when unset or empty.
func webPassword(ctx *Context) *string {
	if password := ctx.EnvValues["PLUGROOT_WEB_PASSWORD"]; password != "" {
		return &password
	}
	return nil
}

// ctEq is equality in time independent of where the inputs differ (the
// length may show).
func ctEq(a, b []byte) bool {
	if len(a) != len(b) {
		return false
	}
	var diff byte
	for i := range a {
		diff |= a[i] ^ b[i]
	}
	return diff == 0
}

func webAuthorized(ctx *Context, req webRequest) bool {
	password := webPassword(ctx)
	if password == nil {
		return true
	}
	expected := "Basic " + base64.StdEncoding.EncodeToString([]byte(webUser(ctx)+":"+*password))
	for _, h := range req.headers {
		if h[0] == "authorization" && ctEq([]byte(h[1]), []byte(expected)) {
			return true
		}
	}
	for _, h := range req.headers {
		if h[0] == "cookie" {
			return webSessionValid(ctx, h[1])
		}
	}
	return false
}

func handleWebLogin(ctx *Context, conn net.Conn, req webRequest) {
	fields := parseFormBody(req.body)
	expectedUser := webUser(ctx)
	expectedPassword := deref(webPassword(ctx), "")
	userOK := ctEq([]byte(fields["user"]), []byte(expectedUser))
	passwordOK := ctEq([]byte(fields["password"]), []byte(expectedPassword))
	if expectedPassword == "" || userOK && passwordOK {
		httpRedirectWithCookie(conn, "/", webSessionCookie(ctx))
		return
	}
	httpResponse(conn, 401, "text/html; charset=utf-8", []byte(renderWebLogin(ctx, "Wrong username or password.")))
}

// parseFormBody reads application/x-www-form-urlencoded pairs; a pair that
// does not decode is dropped, and a later key wins.
func parseFormBody(body string) map[string]string {
	fields := map[string]string{}
	for _, pair := range strings.Split(body, "&") {
		key, value, _ := strings.Cut(pair, "=")
		k, ok1 := urlDecode(key)
		v, ok2 := urlDecode(value)
		if ok1 && ok2 {
			fields[k] = v
		}
	}
	return fields
}

func urlDecode(value string) (string, bool) {
	out := make([]byte, 0, len(value))
	for i := 0; i < len(value); {
		switch c := value[i]; {
		case c == '+':
			out = append(out, ' ')
			i++
		case c == '%' && i+2 < len(value):
			n, ok := parseHexByte(value[i+1 : i+3])
			if !ok {
				return "", false
			}
			out = append(out, n)
			i += 3
		default:
			out = append(out, c)
			i++
		}
	}
	s := string(out)
	return s, utf8.ValidString(s)
}

// parseHexByte is Rust's u8::from_str_radix(s, 16): hex digits with one
// optional leading +.
func parseHexByte(s string) (byte, bool) {
	s = strings.TrimPrefix(s, "+")
	if s == "" || s[0] == '+' || s[0] == '-' {
		return 0, false
	}
	n, err := strconv.ParseUint(s, 16, 8)
	return byte(n), err == nil
}

func webSessionCookie(ctx *Context) string {
	ttl := uint64(604800)
	if n, ok := parseUnsigned(ctx.EnvValues["PLUGROOT_WEB_SESSION_TTL_SECONDS"], 64); ok {
		ttl = n
	}
	expires := nowUnixSeconds()
	if expires > ^uint64(0)-ttl {
		expires = ^uint64(0)
	} else {
		expires += ttl
	}
	return fmt.Sprintf("plugroot_session=%d.%s; Max-Age=%d; Path=/; HttpOnly; SameSite=Lax", expires, webSessionSignature(ctx, expires), ttl)
}

func webSessionValid(ctx *Context, cookieHeader string) bool {
	value, found := "", false
	for _, part := range strings.Split(cookieHeader, ";") {
		name, v, ok := strings.Cut(strings.TrimFunc(part, isRustWhitespace), "=")
		if ok && name == "plugroot_session" {
			value, found = v, true
			break
		}
	}
	if !found {
		return false
	}
	expiresText, signature, ok := strings.Cut(value, ".")
	if !ok {
		return false
	}
	expires, ok := parseUnsigned(expiresText, 64)
	if !ok {
		return false
	}
	return expires >= nowUnixSeconds() && ctEq([]byte(signature), []byte(webSessionSignature(ctx, expires)))
}

// webSessionSignature is HMAC-SHA256 over the user and expiry under the
// session secret; the password never enters the cookie.
func webSessionSignature(ctx *Context, expires uint64) string {
	message := fmt.Sprintf("plugroot-web-session-v2:%s:%d", webUser(ctx), expires)
	return hex.EncodeToString(hmacSHA256(webSessionSecret(ctx), []byte(message)))
}

func hmacSHA256(key, message []byte) []byte {
	mac := hmac.New(sha256.New, key)
	mac.Write(message)
	return mac.Sum(nil)
}

const sessionSecretFile = "web-session.key"

// webSessionSecret is the cookie signing key: 32 random bytes kept as hex
// in a 0600 file under the state root, so sessions survive restarts. When
// that file cannot be written, a per-process secret is used instead.
func webSessionSecret(ctx *Context) []byte {
	path := joinPath(ctx.stateRoot(), sessionSecretFile)
	if secret, ok := readSessionSecret(path); ok {
		return secret
	}
	secret := make([]byte, 32)
	rand.Read(secret)
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err == nil {
		_, err = file.WriteString(hex.EncodeToString(secret) + "\n")
		file.Close()
	}
	switch {
	case err == nil:
		return secret
	case exists(path):
		// Another connection or process wrote it first.
		if secret, ok := readSessionSecret(path); ok {
			return secret
		}
		return processSessionSecret()
	}
	fmt.Fprintf(os.Stderr, "warning: cannot store session secret at %s: %s; sessions end when this process does\n", path, ioError(err))
	return processSessionSecret()
}

func readSessionSecret(path string) ([]byte, bool) {
	text, err := readText(path)
	if err != nil {
		return nil, false
	}
	text = strings.TrimFunc(text, isRustWhitespace)
	if len(text) != 64 || !isASCII(text) {
		return nil, false
	}
	secret := make([]byte, 32)
	for i := range secret {
		b, ok := parseHexByte(text[i*2 : i*2+2])
		if !ok {
			return nil, false
		}
		secret[i] = b
	}
	return secret, true
}

var processSecret = sync.OnceValue(func() []byte {
	secret := make([]byte, 32)
	rand.Read(secret)
	return secret
})

func processSessionSecret() []byte { return processSecret() }

func nowUnixSeconds() uint64 {
	if now := time.Now().Unix(); now > 0 {
		return uint64(now)
	}
	return 0
}

func httpUnauthorized(conn net.Conn) {
	body := "authentication required"
	conn.Write([]byte(fmt.Sprintf("HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: %d\r\nWWW-Authenticate: Basic realm=\"Plugroot\"\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n%s", len(body), body)))
}

func httpRedirect(conn net.Conn, location string) {
	conn.Write([]byte("HTTP/1.1 303 See Other\r\nLocation: " + location + "\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"))
}

func httpRedirectWithCookie(conn net.Conn, location, cookie string) {
	conn.Write([]byte("HTTP/1.1 303 See Other\r\nLocation: " + location + "\r\nSet-Cookie: " + cookie + "\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"))
}

func httpResponse(conn net.Conn, status int, contentType string, body []byte) {
	reason := "OK"
	switch status {
	case 303:
		reason = "See Other"
	case 400:
		reason = "Bad Request"
	case 401:
		reason = "Unauthorized"
	case 403:
		reason = "Forbidden"
	case 404:
		reason = "Not Found"
	case 413:
		reason = "Content Too Large"
	case 500:
		reason = "Internal Server Error"
	}
	head := fmt.Sprintf("HTTP/1.1 %d %s\r\nContent-Type: %s\r\nContent-Length: %d\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", status, reason, contentType, len(body))
	conn.Write(append([]byte(head), body...))
}
