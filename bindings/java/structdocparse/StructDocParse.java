package structdocparse;

import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.NativeLibrary;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;

import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Map;

/**
 * struct_docparse 자바 바인딩 (JNA 5.x).
 *
 * <pre>
 * StructDocParse sdp = StructDocParse.load("lib/struct_docparse.dll", "{\"ocr\":{\"mode\":\"inline\"}}");
 * Map&lt;String,Object&gt; doc = sdp.parse("보고서.hwpx", null);
 * sdp.close();
 * </pre>
 *
 * 문자열은 모두 UTF-8 바이트(끝에 0)로 넘긴다 — JNA 기본 인코딩(운영체제 코드 페이지)에 기대지 않는다.
 */
public final class StructDocParse implements AutoCloseable {

    /** C 함수 목록(include/struct_docparse.h) — 문자열 인자는 byte[](UTF-8 + 0) */
    public interface Api extends Library {
        int sdp_abi_version();
        Pointer sdp_version();
        int sdp_open(byte[] options, Pointer log, Pointer user, PointerByReference out);
        void sdp_close(Pointer h);
        int sdp_set_option(Pointer h, byte[] key, byte[] valueJson);
        int sdp_get_status(Pointer h, PointerByReference out);
        int sdp_supports(Pointer h, byte[] path);
        int sdp_parse(Pointer h, byte[] path, byte[] callOpts, PointerByReference out);
        int sdp_extract_text(Pointer h, byte[] path, int fixTables, PointerByReference out);
        int sdp_ocr_file(Pointer h, byte[] path, byte[] pagesJson, PointerByReference out);
        void sdp_ocr_unload(Pointer h);
        int sdp_pdf_info(Pointer h, byte[] path, PointerByReference out);
        Pointer sdp_last_error();
        void sdp_free(Pointer p);
    }

    /** 라이브러리가 돌려준 오류 — code 는 상태 코드(-1 … -99) */
    public static final class SdpException extends RuntimeException {
        public final int code;

        // 코드와 오류문을 함께 담는다
        public SdpException(int code, String msg) {
            super("[" + code + "] " + msg);
            this.code = code;
        }
    }

    private final Api api;
    private Pointer h;
    private final String libPath;

    // 열린 라이브러리와 핸들을 쥔다
    private StructDocParse(Api api, Pointer h, String libPath) {
        this.api = api;
        this.h = h;
        this.libPath = libPath;
    }

    // 자바 문자열 → UTF-8 바이트 + 끝 0 (null 은 null)
    private static byte[] u8(String s) {
        if (s == null) return null;
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        return Arrays.copyOf(b, b.length + 1);
    }

    /**
     * 라이브러리 열기 + sdp_open
     * -in: libPath = struct_docparse.dll / libstruct_docparse.so 경로
     * -in: optionsJson = 열 때 옵션 JSON(null 이면 기본값)
     * -out: StructDocParse
     * -out: error = 적재 실패 UnsatisfiedLinkError, ABI 다름 · 열기 실패 SdpException
     */
    public static StructDocParse load(String libPath, String optionsJson) {
        File f = new File(libPath).getAbsoluteFile();
        // 라이브러리 폴더를 JNA 검색 경로에 더한다(같은 폴더의 pdfium · onnxruntime 은 라이브러리가 직접 찾는다)
        NativeLibrary.addSearchPath(f.getName(), f.getParent());
        Api api = Native.load(f.getPath(), Api.class);
        if (api.sdp_abi_version() != 1) throw new SdpException(-1, "ABI 가 다릅니다: " + api.sdp_abi_version());
        PointerByReference out = new PointerByReference();
        int rc = api.sdp_open(u8(optionsJson), null, null, out);
        if (rc != 0) throw new SdpException(rc, api.sdp_last_error().getString(0, "UTF-8"));
        return new StructDocParse(api, out.getValue(), f.getPath());
    }

    // 결과 받기 — 0 이면 JSON 글을 읽고 sdp_free, 아니면 예외
    private Object take(int rc, PointerByReference out) {
        if (rc != 0) throw new SdpException(rc, api.sdp_last_error().getString(0, "UTF-8"));
        Pointer p = out.getValue();
        if (p == null) return null;
        try {
            return Json.parse(p.getString(0, "UTF-8"));
        } finally {
            api.sdp_free(p);
        }
    }

    /** 적재한 라이브러리 경로 */
    public String libPath() { return libPath; }

    /** 버전 문자열 — "0.1.0 blocks=5 ocr=…" */
    public String version() { return api.sdp_version().getString(0, "UTF-8"); }

    /** 상태(OCR 켜짐 · 모델 · pdfium) */
    @SuppressWarnings("unchecked")
    public Map<String, Object> status() {
        PointerByReference out = new PointerByReference();
        return (Map<String, Object>) take(api.sdp_get_status(h, out), out);
    }

    /**
     * 문서 파싱
     * -in: path = 파일 경로 / callOptsJson = 호출 옵션 JSON(예: {"ocr":{"enabled":false}}), null 가능
     * -out: 결과 Map(format · title · pages · warnings · raw_text · ocr · blocks)
     * -out: error = 실패면 SdpException(code 로 종류 구분)
     */
    @SuppressWarnings("unchecked")
    public Map<String, Object> parse(String path, String callOptsJson) {
        PointerByReference out = new PointerByReference();
        return (Map<String, Object>) take(api.sdp_parse(h, u8(path), u8(callOptsJson), out), out);
    }

    /** 평문 추출 — {"text", "table_rows"} */
    @SuppressWarnings("unchecked")
    public Map<String, Object> extractText(String path, boolean fixTables) {
        PointerByReference out = new PointerByReference();
        return (Map<String, Object>) take(api.sdp_extract_text(h, u8(path), fixTables ? 1 : 0, out), out);
    }

    /** 실행 중 옵션 바꾸기 — 예: setOption("ocr.enabled", "false") */
    public void setOption(String key, String valueJson) {
        int rc = api.sdp_set_option(h, u8(key), u8(valueJson));
        if (rc != 0) throw new SdpException(rc, api.sdp_last_error().getString(0, "UTF-8"));
    }

    /** 핸들 닫기(OCR 엔진도 내린다) — 두 번 불러도 된다 */
    @Override
    public void close() {
        if (h != null) {
            api.sdp_close(h);
            h = null;
        }
    }
}
