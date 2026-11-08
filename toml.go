package main

// A small, strict TOML reader with the standard library only: the subset
// Plugroot's manifests use. Comments, [table] and
// [[array.of.tables]] headers, bare or quoted keys, "basic" and 'literal'
// strings on one line, decimal integers, booleans and arrays (over several
// lines, with a trailing comma). Anything else (floats, dates, inline
// tables, dotted keys, multi-line strings) is refused with its line number,
// so a file is never half understood.
//
// Tables come back as map[string]any, arrays as []any, strings as string,
// integers as int64 and booleans as bool.

import (
	"errors"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

// tomlTable is a table while it is being read: defined says a header or a
// key already defined it, so a second [header] for it is refused.
type tomlTable struct {
	values  map[string]any
	defined bool
}

// tomlTables is an array of tables, made by [[header]].
type tomlTables struct{ items []*tomlTable }

type tomlReader struct {
	text string
	at   int
	line int
}

// parseTOML reads a whole document.
func parseTOML(text string) (map[string]any, error) {
	r := &tomlReader{text: text, line: 1}
	root := &tomlTable{values: map[string]any{}, defined: true}
	current := root
	for {
		r.skipBlank()
		if r.done() {
			break
		}
		if r.peek() == '[' {
			table, err := r.header(root)
			if err != nil {
				return nil, r.fail(err)
			}
			current = table
			continue
		}
		key, err := r.key()
		if err != nil {
			return nil, r.fail(err)
		}
		r.skipSpace()
		if r.done() || r.peek() != '=' {
			if !r.done() && r.peek() == '.' {
				return nil, r.fail(fmt.Errorf("dotted keys are not read here: %s", key))
			}
			return nil, r.fail(fmt.Errorf("%s needs = and a value", key))
		}
		r.at++
		r.skipSpace()
		value, err := r.value()
		if err != nil {
			return nil, r.fail(fmt.Errorf("%s: %w", key, err))
		}
		if _, taken := current.values[key]; taken {
			return nil, r.fail(fmt.Errorf("duplicate key %s", key))
		}
		current.values[key] = value
		if err := r.endOfLine(); err != nil {
			return nil, r.fail(err)
		}
	}
	return plainTable(root), nil
}

func (r *tomlReader) fail(err error) error {
	return fmt.Errorf("line %d: %w", r.line, err)
}

func (r *tomlReader) done() bool { return r.at >= len(r.text) }
func (r *tomlReader) peek() byte { return r.text[r.at] }

func (r *tomlReader) skipSpace() {
	for !r.done() && (r.peek() == ' ' || r.peek() == '\t') {
		r.at++
	}
}

// skipBlank skips blanks, line ends and comments.
func (r *tomlReader) skipBlank() {
	for !r.done() {
		switch r.peek() {
		case ' ', '\t', '\r':
			r.at++
		case '\n':
			r.at++
			r.line++
		case '#':
			for !r.done() && r.peek() != '\n' {
				r.at++
			}
		default:
			return
		}
	}
}

// endOfLine: only blanks and a comment may follow a value or a header.
func (r *tomlReader) endOfLine() error {
	r.skipSpace()
	if !r.done() && r.peek() == '#' {
		for !r.done() && r.peek() != '\n' {
			r.at++
		}
	}
	if !r.done() && r.peek() == '\r' {
		r.at++
	}
	if !r.done() && r.peek() != '\n' {
		return fmt.Errorf("unexpected %q", r.text[r.at:min(len(r.text), r.at+20)])
	}
	return nil
}

func bareKeyByte(c byte) bool {
	return c == '_' || c == '-' || '0' <= c && c <= '9' || 'a' <= c && c <= 'z' || 'A' <= c && c <= 'Z'
}

// key reads one bare or quoted key.
func (r *tomlReader) key() (string, error) {
	if r.done() {
		return "", errors.New("a key is missing")
	}
	if c := r.peek(); c == '"' || c == '\'' {
		return r.str()
	}
	start := r.at
	for !r.done() && bareKeyByte(r.peek()) {
		r.at++
	}
	if start == r.at {
		return "", fmt.Errorf("a key this reader does not know: %q", r.text[r.at:min(len(r.text), r.at+20)])
	}
	return r.text[start:r.at], nil
}

// header reads [a.b] or [[a.b]] and returns the table that keys now go to.
func (r *tomlReader) header(root *tomlTable) (*tomlTable, error) {
	r.at++
	double := !r.done() && r.peek() == '['
	if double {
		r.at++
	}
	var path []string
	for {
		r.skipSpace()
		key, err := r.key()
		if err != nil {
			return nil, err
		}
		path = append(path, key)
		r.skipSpace()
		if !r.done() && r.peek() == '.' {
			r.at++
			continue
		}
		break
	}
	closing := "]"
	if double {
		closing = "]]"
	}
	if !strings.HasPrefix(r.text[r.at:], closing) {
		return nil, fmt.Errorf("a table header without its %s", closing)
	}
	r.at += len(closing)
	if err := r.endOfLine(); err != nil {
		return nil, err
	}
	name := strings.Join(path, ".")
	table := root
	for _, key := range path[:len(path)-1] {
		switch next := table.values[key].(type) {
		case nil:
			child := &tomlTable{values: map[string]any{}}
			table.values[key] = child
			table = child
		case *tomlTable:
			table = next
		case *tomlTables:
			table = next.items[len(next.items)-1]
		default:
			return nil, fmt.Errorf("%s is not a table", name)
		}
	}
	last := path[len(path)-1]
	existing, taken := table.values[last]
	if double {
		if !taken {
			child := &tomlTable{values: map[string]any{}, defined: true}
			table.values[last] = &tomlTables{items: []*tomlTable{child}}
			return child, nil
		}
		tables, ok := existing.(*tomlTables)
		if !ok {
			return nil, fmt.Errorf("%s is not an array of tables", name)
		}
		child := &tomlTable{values: map[string]any{}, defined: true}
		tables.items = append(tables.items, child)
		return child, nil
	}
	if !taken {
		child := &tomlTable{values: map[string]any{}, defined: true}
		table.values[last] = child
		return child, nil
	}
	child, ok := existing.(*tomlTable)
	if !ok || child.defined {
		return nil, fmt.Errorf("duplicate table %s", name)
	}
	child.defined = true
	return child, nil
}

func (r *tomlReader) value() (any, error) {
	if r.done() {
		return nil, errors.New("no value")
	}
	switch c := r.peek(); {
	case c == '"' || c == '\'':
		return r.str()
	case c == '{':
		return nil, errors.New("inline tables are not read here")
	case c == '[':
		r.at++
		list := []any{}
		for {
			r.skipBlank()
			if r.done() {
				return nil, errors.New("a list without its ]")
			}
			if r.peek() == ']' {
				r.at++
				return list, nil
			}
			item, err := r.value()
			if err != nil {
				return nil, err
			}
			list = append(list, item)
			r.skipBlank()
			if !r.done() && r.peek() == ',' {
				r.at++
			} else if r.done() || r.peek() != ']' {
				return nil, errors.New("list items are separated by commas")
			}
		}
	}
	start := r.at
	for !r.done() && !strings.ContainsRune(" \t\r\n,]#", rune(r.peek())) {
		r.at++
	}
	word := r.text[start:r.at]
	switch word {
	case "true":
		return true, nil
	case "false":
		return false, nil
	}
	digits := strings.TrimLeft(word, "+-")
	if digits == "" || len(word)-len(digits) > 1 || digits[0] == '_' || digits[len(digits)-1] == '_' ||
		strings.Contains(digits, "__") || len(digits) > 1 && digits[0] == '0' {
		return nil, fmt.Errorf("a value this reader does not know: %q", word)
	}
	for i := 0; i < len(digits); i++ {
		if digits[i] != '_' && (digits[i] < '0' || digits[i] > '9') {
			return nil, fmt.Errorf("a value this reader does not know: %q", word)
		}
	}
	n, err := strconv.ParseInt(strings.ReplaceAll(word, "_", ""), 10, 64)
	if err != nil {
		return nil, fmt.Errorf("an integer out of range: %q", word)
	}
	return n, nil
}

// str reads a "basic" string with its escapes or a 'literal' one, on one
// line.
func (r *tomlReader) str() (string, error) {
	quote := r.peek()
	if strings.HasPrefix(r.text[r.at:], strings.Repeat(string(quote), 3)) {
		return "", errors.New("multi-line strings are not read here")
	}
	r.at++
	var b strings.Builder
	for !r.done() {
		c := r.peek()
		r.at++
		switch {
		case c == quote:
			if !utf8.ValidString(b.String()) {
				return "", errors.New("a string that is not UTF-8")
			}
			return b.String(), nil
		case c == '\n':
			return "", errors.New("a string ends on its line")
		case c == '\\' && quote == '"':
			if r.done() {
				return "", errors.New("a string without its end")
			}
			e := r.peek()
			r.at++
			switch e {
			case 'b':
				b.WriteByte('\b')
			case 't':
				b.WriteByte('\t')
			case 'n':
				b.WriteByte('\n')
			case 'f':
				b.WriteByte('\f')
			case 'r':
				b.WriteByte('\r')
			case '"', '\\':
				b.WriteByte(e)
			case 'u', 'U':
				size := 4
				if e == 'U' {
					size = 8
				}
				if r.at+size > len(r.text) {
					return "", errors.New("a short \\u escape")
				}
				code, err := strconv.ParseUint(r.text[r.at:r.at+size], 16, 32)
				if err != nil || !utf8.ValidRune(rune(code)) {
					return "", errors.New("a bad \\u escape")
				}
				b.WriteRune(rune(code))
				r.at += size
			default:
				return "", fmt.Errorf("an unknown escape \\%c", e)
			}
		case c < 0x20 && c != '\t' || c == 0x7f:
			return "", errors.New("a control character in a string")
		default:
			b.WriteByte(c)
		}
	}
	return "", errors.New("a string without its end")
}

// plainTable turns the reader's tables into maps and arrays.
func plainTable(t *tomlTable) map[string]any {
	out := make(map[string]any, len(t.values))
	for key, value := range t.values {
		switch v := value.(type) {
		case *tomlTable:
			out[key] = plainTable(v)
		case *tomlTables:
			items := make([]any, len(v.items))
			for i, item := range v.items {
				items[i] = plainTable(item)
			}
			out[key] = items
		default:
			out[key] = value
		}
	}
	return out
}
