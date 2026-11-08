package main

// Pretty JSON exactly as the Rust version's serde_json wrote it: two-space
// indent, "key": value, [] and {} when empty, and serde_json's escapes
// (no HTML escaping, control characters as \b \f \n \r \t or \u00xx).

import (
	"strconv"
	"strings"
)

// jsonObject keeps its fields in order.
type jsonObject []jsonField

type jsonField struct {
	key   string
	value any
}

func prettyJSON(value any) string {
	var b strings.Builder
	writeJSON(&b, value, 0)
	return b.String()
}

func writeJSON(b *strings.Builder, value any, depth int) {
	switch v := value.(type) {
	case nil:
		b.WriteString("null")
	case string:
		writeJSONString(b, v)
	case *string:
		if v == nil {
			b.WriteString("null")
		} else {
			writeJSONString(b, *v)
		}
	case bool:
		b.WriteString(strconv.FormatBool(v))
	case int:
		b.WriteString(strconv.Itoa(v))
	case int64:
		b.WriteString(strconv.FormatInt(v, 10))
	case uint64:
		b.WriteString(strconv.FormatUint(v, 10))
	case []string:
		items := make([]any, len(v))
		for i, s := range v {
			items[i] = s
		}
		writeJSON(b, items, depth)
	case []any:
		if len(v) == 0 {
			b.WriteString("[]")
			return
		}
		b.WriteString("[\n")
		for i, item := range v {
			b.WriteString(strings.Repeat("  ", depth+1))
			writeJSON(b, item, depth+1)
			if i < len(v)-1 {
				b.WriteByte(',')
			}
			b.WriteByte('\n')
		}
		b.WriteString(strings.Repeat("  ", depth) + "]")
	case jsonObject:
		if len(v) == 0 {
			b.WriteString("{}")
			return
		}
		b.WriteString("{\n")
		for i, field := range v {
			b.WriteString(strings.Repeat("  ", depth+1))
			writeJSONString(b, field.key)
			b.WriteString(": ")
			writeJSON(b, field.value, depth+1)
			if i < len(v)-1 {
				b.WriteByte(',')
			}
			b.WriteByte('\n')
		}
		b.WriteString(strings.Repeat("  ", depth) + "}")
	default:
		panic("prettyJSON: unsupported type")
	}
}

func writeJSONString(b *strings.Builder, s string) {
	const hex = "0123456789abcdef"
	b.WriteByte('"')
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch c {
		case '"':
			b.WriteString(`\"`)
		case '\\':
			b.WriteString(`\\`)
		case '\n':
			b.WriteString(`\n`)
		case '\r':
			b.WriteString(`\r`)
		case '\t':
			b.WriteString(`\t`)
		case '\b':
			b.WriteString(`\b`)
		case '\f':
			b.WriteString(`\f`)
		default:
			if c < 0x20 {
				b.WriteString(`\u00`)
				b.WriteByte(hex[c>>4])
				b.WriteByte(hex[c&0xf])
			} else {
				b.WriteByte(c)
			}
		}
	}
	b.WriteByte('"')
}
