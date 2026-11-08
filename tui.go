package main

// plugroot tui: a terminal dashboard drawn with plain escape codes (the
// same ones the Rust version's crossterm wrote) in raw mode.

import (
	"bufio"
	"errors"
	"fmt"
	"os"
	"strings"
	"syscall"
	"time"
	"unsafe"
)

const tuiHeader = "  STATE    PLANE    CATEGORY/KIND  SERVICE                         ACCESS"

type tuiApp struct {
	ctx      *Context
	rows     []StatusRow
	selected int
	scrollX  int
	scrollY  int
	message  string
}

func newTUIApp(ctx *Context) *tuiApp {
	return &tuiApp{ctx: ctx, rows: statusRows(ctx)}
}

func cmdTUI(ctx *Context, args []string) (int, error) {
	for _, arg := range args {
		if arg == "--once" {
			printStatusTable(statusRows(ctx))
			return 0, nil
		}
	}
	app := newTUIApp(ctx)
	term, err := enterTerminal()
	if err != nil {
		return 0, err
	}
	defer term.leave()
	keys := make(chan string)
	go term.readKeys(keys)
	for {
		if err := drawTUI(term, app); err != nil {
			return 0, err
		}
		select {
		case key, ok := <-keys:
			if !ok {
				return 0, nil
			}
			switch key {
			case "q", "esc":
				return 0, nil
			case "down", "j":
				app.down()
			case "up", "k":
				app.up()
			case "right":
				app.right()
			case "left":
				app.left()
			case "pagedown":
				_, height := term.size()
				app.pageDown(tuiListCapacity(height))
			case "pageup":
				_, height := term.size()
				app.pageUp(tuiListCapacity(height))
			case "home":
				app.home()
			case "end":
				app.end()
			case "r":
				app.refresh()
			case "l":
				app.action("logs")
			case "o":
				app.action("start")
			case "e":
				app.action("restart")
			case "f":
				app.action("stop")
			}
		case <-time.After(500 * time.Millisecond):
		}
	}
}

// The escape codes crossterm wrote.
func moveTo(x, y int) string { return fmt.Sprintf("\x1b[%d;%dH", y+1, x+1) }

const (
	clearAll    = "\x1b[2J"
	bold        = "\x1b[1m"
	reverse     = "\x1b[7m"
	resetAttr   = "\x1b[0m"
	resetColor  = "\x1b[0m"
	darkGrey    = "\x1b[38;5;8m"
	enterAlt    = "\x1b[?1049h"
	leaveAlt    = "\x1b[?1049l"
	hideCursor  = "\x1b[?25l"
	showCursor  = "\x1b[?25h"
	colorGreen  = "\x1b[38;5;10m"
	colorRed    = "\x1b[38;5;9m"
	colorYellow = "\x1b[38;5;11m"
	colorWhite  = "\x1b[38;5;15m"
)

func colorForState(state string) string {
	switch state {
	case "online":
		return colorGreen
	case "offline", "missing":
		return colorRed
	case "partial", "unknown":
		return colorYellow
	}
	return colorWhite
}

func drawTUI(term *terminal, app *tuiApp) error {
	terminalWidth, height := term.size()
	width := terminalWidth
	available := tuiListCapacity(height)
	app.clampViewport(available, width)

	var b strings.Builder
	b.WriteString(moveTo(0, 0) + clearAll)
	online := 0
	for _, row := range app.rows {
		if row.State == "online" {
			online++
		}
	}
	name := deref(app.ctx.hostField(func(h *HostConfig) *string { return h.Name }), "plugroot")
	ip := deref(app.ctx.hostField(func(h *HostConfig) *string { return h.PrivateIP }), "unknown")
	user := deref(app.ctx.hostField(func(h *HostConfig) *string { return h.User }), "-")
	b.WriteString(bold + fit(fmt.Sprintf("Plugroot - %s - %s - user %s", name, ip, user), width) + resetAttr)
	b.WriteString(moveTo(0, 1) + fit(fmt.Sprintf("%d online, %d attention, %d tracked - arrows move/pan - r refresh - l logs - o on - e restart - f off - q quit",
		online, len(app.rows)-online, len(app.rows)), width))
	b.WriteString(moveTo(0, 2) + strings.Repeat("-", terminalWidth))

	const listTop = 3
	start, end := visibleRows(app, available)
	b.WriteString(moveTo(0, listTop) + darkGrey + fitWindow(tuiHeader, width, app.scrollX) + resetColor)
	for actual := start; actual < end; actual++ {
		row := app.rows[actual]
		y := listTop + 1 + (actual - start)
		marker := " "
		if actual == app.selected {
			marker = ">"
		}
		text := tuiRowText(marker, row, row.Category+"/"+row.Kind, rowAccess(row))
		b.WriteString(moveTo(0, y))
		if actual == app.selected {
			b.WriteString(reverse)
		}
		b.WriteString(colorForState(row.State) + fitWindow(text, width, app.scrollX) + resetColor)
		if actual == app.selected {
			b.WriteString(resetAttr)
		}
	}
	base := max(height-5, 0)
	b.WriteString(moveTo(0, base) + strings.Repeat("-", terminalWidth))
	if app.selected < len(app.rows) {
		row := app.rows[app.selected]
		description := row.Detail
		if row.Description != nil {
			description = *row.Description
		}
		b.WriteString(moveTo(0, base+1) + bold + fit(fmt.Sprintf("%s (%s)", row.Name, row.ID), width) + resetAttr)
		b.WriteString(moveTo(0, base+2) + fit(description, width))
		b.WriteString(moveTo(0, base+3) + fitWindow(tuiDetailText(row, rowAccess(row)), width, app.scrollX))
		b.WriteString(moveTo(0, base+4) + fitWindow(app.message, width, app.scrollX))
	}
	_, err := term.out.WriteString(b.String())
	if err == nil {
		err = term.out.Flush()
	}
	return err
}

func rowAccess(row StatusRow) string {
	if row.URL != nil {
		return *row.URL
	}
	if row.Access != nil {
		return *row.Access
	}
	return "-"
}

func tuiListCapacity(height int) int {
	const listTop = 3
	footerStart := max(height-5, 0)
	return max(footerStart-(listTop+1), 0)
}

func visibleRows(app *tuiApp, available int) (int, int) {
	return app.scrollY, min(app.scrollY+available, len(app.rows))
}

func tuiRowText(marker string, row StatusRow, categoryKind, access string) string {
	return fmt.Sprintf("%s %-8s %-8s %-14s %-30s %s", marker, row.State, row.Plane, categoryKind, row.Name, access)
}

func tuiContentWidth(app *tuiApp) int {
	widest := visibleWidth(tuiHeader)
	for _, row := range app.rows {
		access := rowAccess(row)
		widest = max(widest, visibleWidth(tuiRowText(">", row, row.Category+"/"+row.Kind, access)))
		widest = max(widest, visibleWidth(tuiDetailText(row, access)))
	}
	return widest
}

func tuiDetailText(row StatusRow, access string) string {
	ports, controls := "none", "none"
	if len(row.Ports) > 0 {
		ports = strings.Join(row.Ports, ",")
	}
	if len(row.Controls) > 0 {
		controls = strings.Join(row.Controls, ",")
	}
	return fmt.Sprintf("access: %s | detail: %s | ports: %s | controls: %s", access, row.Detail, ports, controls)
}

// fit is value on one line, cut with ~ and padded to width characters.
func fit(value string, width int) string {
	if width == 0 {
		return ""
	}
	text := []rune(strings.ReplaceAll(value, "\n", " "))
	if len(text) > width {
		text = append(text[:width-1], '~')
	}
	return padRight(string(text), width)
}

// fitWindow is width characters of value from offset, with < and > where
// more text lies to the left or right.
func fitWindow(value string, width, offset int) string {
	if width == 0 {
		return ""
	}
	chars := []rune(strings.ReplaceAll(value, "\n", " "))
	offset = min(offset, max(len(chars)-1, 0))
	window := append([]rune{}, chars[min(offset, len(chars)):min(offset+width, len(chars))]...)
	if offset > 0 && len(window) > 0 {
		window[0] = '<'
	}
	if offset+width < len(chars) && len(window) > 0 {
		window[len(window)-1] = '>'
	}
	return padRight(string(window), width)
}

func padRight(s string, width int) string {
	if n := len([]rune(s)); n < width {
		return s + strings.Repeat(" ", width-n)
	}
	return s
}

func visibleWidth(value string) int {
	return len([]rune(strings.ReplaceAll(value, "\n", " ")))
}

// trimOutput keeps the last maxChars characters of trimmed output.
func trimOutput(value string, maxChars int) string {
	chars := []rune(strings.TrimSpace(value))
	if len(chars) <= maxChars {
		return string(chars)
	}
	return "[truncated]\n" + string(chars[len(chars)-maxChars:])
}

func (a *tuiApp) refresh() {
	selectedID, hasSelected := "", a.selected < len(a.rows)
	if hasSelected {
		selectedID = a.rows[a.selected].ID
	}
	selectedIndex := a.selected
	ctx, err := loadContext(a.ctx.Root)
	if err != nil {
		a.message = fmt.Sprintf("refresh failed: %s", err)
		return
	}
	a.ctx = ctx
	a.rows = statusRows(ctx)
	a.restoreSelection(selectedID, hasSelected, selectedIndex)
}

func (a *tuiApp) restoreSelection(id string, hasID bool, index int) {
	if len(a.rows) == 0 {
		a.selected, a.scrollY = 0, 0
		return
	}
	if hasID {
		for i, row := range a.rows {
			if row.ID == id {
				a.selected = i
				return
			}
		}
	}
	a.selected = min(index, len(a.rows)-1)
}

func (a *tuiApp) up()    { a.selected = max(a.selected-1, 0) }
func (a *tuiApp) right() { a.scrollX += 8 }
func (a *tuiApp) left()  { a.scrollX = max(a.scrollX-8, 0) }

func (a *tuiApp) down() {
	if a.selected+1 < len(a.rows) {
		a.selected++
	}
}

func (a *tuiApp) pageDown(visible int) {
	if len(a.rows) == 0 {
		return
	}
	a.selected = min(a.selected+max(visible, 1), len(a.rows)-1)
}

func (a *tuiApp) pageUp(visible int) { a.selected = max(a.selected-max(visible, 1), 0) }

func (a *tuiApp) home() { a.selected, a.scrollY, a.scrollX = 0, 0, 0 }

func (a *tuiApp) end() {
	if len(a.rows) > 0 {
		a.selected = len(a.rows) - 1
	}
}

func (a *tuiApp) clampViewport(visible, width int) {
	if len(a.rows) == 0 {
		a.selected, a.scrollY, a.scrollX = 0, 0, 0
		return
	}
	visible = max(visible, 1)
	a.selected = min(a.selected, len(a.rows)-1)
	if a.selected < a.scrollY {
		a.scrollY = a.selected
	} else if a.selected >= a.scrollY+visible {
		a.scrollY = a.selected + 1 - visible
	}
	a.scrollY = min(a.scrollY, max(len(a.rows)-visible, 0))
	a.scrollX = min(a.scrollX, max(tuiContentWidth(a)-width, 0))
}

func (a *tuiApp) action(action string) {
	if a.selected >= len(a.rows) {
		return
	}
	s, ok := a.ctx.service(a.rows[a.selected].ID)
	if !ok {
		return
	}
	if !hasControl(s, action) {
		a.message = fmt.Sprintf("%s does not allow `%s`", s.Name, action)
		return
	}
	var out CmdOutput
	if action == "logs" {
		out = logsFor(a.ctx, s)
	} else {
		out = runFixedAction(a.ctx, s, action)
	}
	a.message = fmt.Sprintf("%s %s: exit %d\n%s", s.Name, action, out.Code, trimOutput(out.Text, 2200))
	a.refresh()
}

// terminal is the controlling terminal in raw mode on the alternate screen.
type terminal struct {
	in       *os.File
	out      *bufio.Writer
	stdout   *os.File
	original syscall.Termios
	closeIn  bool
}

// enterTerminal switches the terminal (standard input when it is one, else
// /dev/tty) to raw mode, as cfmakeraw does, and shows the alternate screen.
func enterTerminal() (*terminal, error) {
	in, closeIn := os.Stdin, false
	if !isTerminal(in.Fd()) {
		tty, err := os.OpenFile("/dev/tty", os.O_RDWR, 0)
		if err != nil {
			return nil, ioError(err)
		}
		in, closeIn = tty, true
	}
	t := &terminal{in: in, out: bufio.NewWriter(os.Stdout), stdout: os.Stdout, closeIn: closeIn}
	if err := ioctlTermios(in.Fd(), syscall.TCGETS, &t.original); err != nil {
		return nil, ioError(err)
	}
	raw := t.original
	raw.Iflag &^= syscall.IGNBRK | syscall.BRKINT | syscall.PARMRK | syscall.ISTRIP | syscall.INLCR | syscall.IGNCR | syscall.ICRNL | syscall.IXON
	raw.Oflag &^= syscall.OPOST
	raw.Lflag &^= syscall.ECHO | syscall.ECHONL | syscall.ICANON | syscall.ISIG | syscall.IEXTEN
	raw.Cflag &^= syscall.CSIZE | syscall.PARENB
	raw.Cflag |= syscall.CS8
	raw.Cc[syscall.VMIN], raw.Cc[syscall.VTIME] = 1, 0
	if err := ioctlTermios(in.Fd(), syscall.TCSETS, &raw); err != nil {
		return nil, ioError(err)
	}
	t.out.WriteString(enterAlt + hideCursor)
	t.out.Flush()
	return t, nil
}

func (t *terminal) leave() {
	t.out.WriteString(showCursor + leaveAlt + resetColor)
	t.out.Flush()
	ioctlTermios(t.in.Fd(), syscall.TCSETS, &t.original)
	if t.closeIn {
		t.in.Close()
	}
}

// size is the terminal's columns and rows, from /dev/tty or standard
// output, else from tput as crossterm did.
func (t *terminal) size() (int, int) {
	if tty, err := os.Open("/dev/tty"); err == nil {
		cols, rows, ok := windowSize(tty.Fd())
		tty.Close()
		if ok {
			return cols, rows
		}
	}
	if cols, rows, ok := windowSize(t.stdout.Fd()); ok {
		return cols, rows
	}
	cols, okc := commandText("tput", "cols")
	rows, okr := commandText("tput", "lines")
	c, ok1 := parseUnsigned(cols, 16)
	r, ok2 := parseUnsigned(rows, 16)
	if okc && okr && ok1 && ok2 {
		return int(c), int(r)
	}
	return 0, 0
}

// readKeys turns input bytes into key names; a lone escape is "esc".
func (t *terminal) readKeys(keys chan<- string) {
	defer close(keys)
	buf := make([]byte, 64)
	for {
		n, err := t.in.Read(buf)
		if err != nil {
			if errors.Is(err, syscall.EINTR) {
				continue
			}
			return
		}
		for _, key := range parseKeys(buf[:n]) {
			keys <- key
		}
	}
}

// parseKeys reads the keys one read returned.
func parseKeys(data []byte) []string {
	var keys []string
	for i := 0; i < len(data); {
		if data[i] != 0x1b {
			keys = append(keys, string(data[i]))
			i++
			continue
		}
		if i+1 >= len(data) {
			keys = append(keys, "esc")
			i++
			continue
		}
		if data[i+1] != '[' && data[i+1] != 'O' {
			keys = append(keys, "esc")
			i++
			continue
		}
		j := i + 2
		for j < len(data) && (data[j] >= '0' && data[j] <= '9' || data[j] == ';') {
			j++
		}
		if j >= len(data) {
			keys = append(keys, "esc")
			i = len(data)
			continue
		}
		seq := string(data[i+2 : j+1])
		i = j + 1
		switch seq {
		case "A":
			keys = append(keys, "up")
		case "B":
			keys = append(keys, "down")
		case "C":
			keys = append(keys, "right")
		case "D":
			keys = append(keys, "left")
		case "H", "1~", "7~":
			keys = append(keys, "home")
		case "F", "4~", "8~":
			keys = append(keys, "end")
		case "5~":
			keys = append(keys, "pageup")
		case "6~":
			keys = append(keys, "pagedown")
		}
	}
	return keys
}

func isTerminal(fd uintptr) bool {
	var t syscall.Termios
	return ioctlTermios(fd, syscall.TCGETS, &t) == nil
}

func ioctlTermios(fd uintptr, request uintptr, t *syscall.Termios) error {
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, fd, request, uintptr(unsafe.Pointer(t)))
	if errno != 0 {
		return errno
	}
	return nil
}

func windowSize(fd uintptr) (int, int, bool) {
	var ws struct{ rows, cols, xpixel, ypixel uint16 }
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, fd, syscall.TIOCGWINSZ, uintptr(unsafe.Pointer(&ws)))
	if errno != 0 {
		return 0, 0, false
	}
	return int(ws.cols), int(ws.rows), true
}
