package structdocparse;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * 아주 작은 JSON 해석기 — 라이브러리 결과(JSON)를 Map · List · String · Long · Double · Boolean · null 로 바꾼다.
 * 외부 라이브러리 없이 예제를 돌리려고 넣었다. 이미 Jackson · Gson 을 쓰면 그것으로 읽어도 된다.
 */
public final class Json {
    private final String s;
    private int i;

    // 해석할 글을 쥔다
    private Json(String s) {
        this.s = s;
    }

    /**
     * JSON 글 → 자바 값
     * -in: text = JSON 글
     * -out: Map / List / String / Long / Double / Boolean / null
     * -out: error = 모양이 틀리면 IllegalArgumentException
     */
    public static Object parse(String text) {
        Json p = new Json(text);
        p.ws();
        Object v = p.value();
        p.ws();
        if (p.i != p.s.length()) throw new IllegalArgumentException("JSON 끝에 남은 글: " + p.i);
        return v;
    }

    // 공백 건너뛰기
    private void ws() {
        while (i < s.length() && Character.isWhitespace(s.charAt(i))) i++;
    }

    // 값 하나 읽기(첫 글자로 종류를 고른다)
    private Object value() {
        char c = s.charAt(i);
        if (c == '{') return object();
        if (c == '[') return array();
        if (c == '"') return string();
        if (s.startsWith("true", i)) { i += 4; return Boolean.TRUE; }
        if (s.startsWith("false", i)) { i += 5; return Boolean.FALSE; }
        if (s.startsWith("null", i)) { i += 4; return null; }
        return number();
    }

    // 객체 { "키": 값, ... } — 들어온 순서를 지킨다
    private Map<String, Object> object() {
        Map<String, Object> m = new LinkedHashMap<>();
        i++;
        ws();
        if (s.charAt(i) == '}') { i++; return m; }
        while (true) {
            ws();
            String k = string();
            ws();
            i++; // ':'
            ws();
            m.put(k, value());
            ws();
            char c = s.charAt(i++);
            if (c == '}') return m;
        }
    }

    // 배열 [ 값, ... ]
    private List<Object> array() {
        List<Object> a = new ArrayList<>();
        i++;
        ws();
        if (s.charAt(i) == ']') { i++; return a; }
        while (true) {
            ws();
            a.add(value());
            ws();
            char c = s.charAt(i++);
            if (c == ']') return a;
        }
    }

    // 문자열 "…" — \n \t \" \\ \\uXXXX(서로게이트 쌍 포함) 풀기
    private String string() {
        StringBuilder b = new StringBuilder();
        i++;
        while (true) {
            char c = s.charAt(i++);
            if (c == '"') return b.toString();
            if (c != '\\') { b.append(c); continue; }
            char e = s.charAt(i++);
            switch (e) {
                case 'n': b.append('\n'); break;
                case 't': b.append('\t'); break;
                case 'r': b.append('\r'); break;
                case 'b': b.append('\b'); break;
                case 'f': b.append('\f'); break;
                case 'u': b.append((char) Integer.parseInt(s.substring(i, i + 4), 16)); i += 4; break;
                default: b.append(e);
            }
        }
    }

    // 숫자 — 소수점·지수가 있으면 Double, 아니면 Long
    private Object number() {
        int st = i;
        while (i < s.length() && "+-0123456789.eE".indexOf(s.charAt(i)) >= 0) i++;
        String t = s.substring(st, i);
        if (t.contains(".") || t.contains("e") || t.contains("E")) return Double.parseDouble(t);
        return Long.parseLong(t);
    }

    /** 자바 문자열 → JSON 문자열 글("…") — 옵션 JSON 을 만들 때 쓴다 */
    public static String quote(String v) {
        StringBuilder b = new StringBuilder("\"");
        for (char c : v.toCharArray()) {
            if (c == '"' || c == '\\') b.append('\\').append(c);
            else if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
            else b.append(c);
        }
        return b.append('"').toString();
    }
}
