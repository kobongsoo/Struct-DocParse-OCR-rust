import structdocparse.StructDocParse;

import java.io.File;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * struct_docparse 자바 예제 — 문서마다 형식 · 라벨별 블록 수 · 앞 블록 3개.
 *
 * <pre>
 * (examples/java 폴더에서)
 * javac -encoding UTF-8 -cp ../../third_party/java/jna-5.14.0.jar -d out ../../bindings/java/structdocparse/*.java Example.java
 * java  -cp "../../third_party/java/jna-5.14.0.jar;out" Example 문서1 [문서2 ...]      (Linux 는 ';' 대신 ':')
 * </pre>
 * 라이브러리 경로: 시스템 속성 sdp.lib 또는 환경변수 SDP_LIB → 묶음 lib/ → 저장소 target/release
 */
public class Example {
    // 운영체제별 라이브러리 파일 이름
    static final String LIB = System.getProperty("os.name").toLowerCase().contains("win") ? "struct_docparse.dll" : "libstruct_docparse.so";

    /**
     * 라이브러리 경로 찾기
     * -out: 경로 (못 찾으면 종료)
     */
    static String findLib() {
        String[] cands = {System.getProperty("sdp.lib", ""), System.getenv().getOrDefault("SDP_LIB", ""),
                "../../lib/" + LIB, "../../target/release/" + LIB};
        for (String c : cands) {
            if (!c.isEmpty() && new File(c).isFile()) return c;
        }
        System.err.println("라이브러리를 찾지 못했습니다 — -Dsdp.lib=경로 를 주세요");
        System.exit(2);
        return null;
    }

    /**
     * 문서 하나 파싱해 요약 출력
     * -in: sdp = 열린 라이브러리 / path = 문서 / callOpts = 호출 옵션 JSON(null 가능)
     */
    @SuppressWarnings("unchecked")
    static void show(StructDocParse sdp, String path, String callOpts) {
        String name = new File(path).getName();
        try {
            Map<String, Object> d = sdp.parse(path, callOpts);
            List<Map<String, Object>> blocks = (List<Map<String, Object>>) d.get("blocks");
            // 라벨별 개수
            Map<String, Integer> kinds = new LinkedHashMap<>();
            for (Map<String, Object> b : blocks) kinds.merge((String) b.get("type"), 1, Integer::sum);
            System.out.printf("  %s : %s · 블록 %d · %s%n", name, d.get("format"), blocks.size(), kinds);
            for (int i = 0; i < Math.min(3, blocks.size()); i++) {
                String t = ((String) blocks.get(i).get("text")).replace('\n', ' ');
                System.out.printf("      [%s] %s%n", blocks.get(i).get("type"), t.length() > 50 ? t.substring(0, 50) : t);
            }
        } catch (StructDocParse.SdpException e) {
            System.out.printf("  %s : 실패 %s%n", name, e.getMessage());
        }
    }

    public static void main(String[] args) {
        try (StructDocParse sdp = StructDocParse.load(findLib(), "{\"ocr\":{\"enabled\":true,\"mode\":\"inline\"}}")) {
            Map<String, Object> st = sdp.status();
            System.out.println("struct_docparse " + sdp.version() + " · OCR " + ((Map<?, ?>) st.get("ocr")).get("enabled"));
            for (String a : args) show(sdp, a, null);
            // 이번 호출만 OCR 끄기 — 그림이면 [-4] 가 정상
            if (args.length > 0) {
                System.out.println("OCR 끔으로 첫 문서 다시:");
                show(sdp, args[0], "{\"ocr\":{\"enabled\":false}}");
            }
        }
    }
}
