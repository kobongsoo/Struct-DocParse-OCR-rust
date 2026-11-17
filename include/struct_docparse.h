/*------------------------------------------------------------------
 * struct_docparse.h — 구조화 문서 파서 + OCR DLL (ABI 1)
 *=> 파일 하나 → 블록 목록(글 + 라벨 + 제목 수준 + 쪽 + 표 칸 + 출처) JSON.
 *   · 문자열은 모두 UTF-8, NUL 끝
 *   · DLL 이 돌려준 char* 는 반드시 sdp_free 로 돌려준다(sdp_version · sdp_last_error 는 빼고)
 *   · 반환값은 상태 코드(SDP_OK = 0), 결과는 out 포인터로
 *   · 핸들 하나를 여러 스레드가 같이 써도 된다
 *   결과 JSON 모양 · 옵션은 docs/API.md 와 SimpleRAG plan/구조화문서파서_DLL분리_설계서_20261116.html §6 · §7
 *------------------------------------------------------------------*/
#ifndef STRUCT_DOCPARSE_H
#define STRUCT_DOCPARSE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* 상태 코드 */
#define SDP_OK             0
#define SDP_EINVAL        -1   /* 널 포인터 · 잘못된 JSON · 모르는 옵션 */
#define SDP_EIO           -2   /* 파일을 열 수 없음 */
#define SDP_EPARSE        -3   /* 구조 파서와 폴백 추출이 모두 실패 */
#define SDP_EUNSUPPORTED  -4   /* 지원하지 않는 형식(OCR 꺼진 상태의 그림 파일 포함) */
#define SDP_EOCR          -5   /* OCR 을 쓸 수 없거나 OCR 중 실패 */
#define SDP_ECANCELLED    -6   /* (예약) */
#define SDP_ERUNTIME      -7   /* pdfium.dll · onnxruntime.dll 적재 실패 */
#define SDP_EPANIC       -99   /* DLL 안의 panic 을 잡음(프로세스는 살아 있음) */

typedef struct SdpHandle SdpHandle;

/* 로그 콜백: level 1 = 경고, 2 = 정보 */
typedef void (*sdp_log_fn)(int32_t level, const char* msg, void* user);
/* 진행 콜백: k(1부터) / 전체 쪽 수 / 그 쪽에 걸린 ms. 0 이 아닌 값을 돌려주면 그 쪽까지 하고 선다 */
typedef int32_t (*sdp_progress_fn)(int32_t page_index, int32_t page_count, double ms, void* user);

uint32_t    sdp_abi_version(void);                 /* 1 */
const char* sdp_version(void);                     /* "0.1.0 blocks=6 ocr=…" — 해제하지 않음 */

/* 열기 — options_json 예:
 *   {"ocr":{"enabled":true,"images":true,"mode":"inline"|"deferred","model_dir":"…","cache_dir":"…","threads":4,"max_pages":100,"layout":true,
 *            "arena":true,"page_cap":"a4"|"none","image_cap":false,"page_up":true,"orient":true,"orient_flip":false},
 *           (arena · page_cap · image_cap · page_up · orient · orient_flip 는 0.1.1 부터)
 *    "pdf":{"tables":true,"infer_headings":false}, "runtime":{"pdfium":"…","onnxruntime":"…"}, "log":{"level":"warn"}}
 *   널이면 기본값(OCR 켬 · inline). 옵션은 프로세스에 한 벌 — 핸들 여러 개는 마지막 옵션을 함께 쓴다 */
int32_t sdp_open(const char* options_json, sdp_log_fn log, void* user, SdpHandle** out);
void    sdp_close(SdpHandle* h);
int32_t sdp_set_option(SdpHandle* h, const char* key, const char* value_json);   /* 예: "ocr.enabled", "false" */
int32_t sdp_get_status(SdpHandle* h, char** out_json);

int32_t sdp_supports(SdpHandle* h, const char* path);  /* 1 블록 경로 / 2 폴백 평문 / 0 미지원 / 음수 오류 */
/* call_opts_json: {"title":"…", "keep_pages":[0,5,9], "ocr":{"enabled":false}} — 모두 생략 가능(널 가능) */
int32_t sdp_parse(SdpHandle* h, const char* path, const char* call_opts_json, char** out_json);
int32_t sdp_extract_text(SdpHandle* h, const char* path, int32_t fix_tables, char** out_json);  /* {"text","table_rows"} */

/* 쪽 OCR → 캐시. pages_json = [쪽 번호(1부터)] 또는 널(남은 쪽 전부). 결과 {"pages","skipped","stopped"} */
int32_t sdp_ocr_pages(SdpHandle* h, const char* path, const char* pages_json,
                      sdp_progress_fn cb, void* user, char** out_json);
/* 파일(PDF · 그림 · 그림 폴더) OCR 시험 — 캐시를 쓰지 않고 쪽마다 줄 · 점수 · 상자 · 시간 */
int32_t sdp_ocr_file(SdpHandle* h, const char* path, const char* pages_json, char** out_json);
void    sdp_ocr_unload(SdpHandle* h);
int32_t sdp_ocr_cache_remove(SdpHandle* h, const char* content_hash);

int32_t sdp_pdf_info(SdpHandle* h, const char* path, char** out_json);        /* {"info":[[키,값]],"pages","toc","text_chars"} */
int32_t sdp_pdf_page_texts(SdpHandle* h, const char* path, char** out_json);  /* {"pages":[쪽 글…]} */

const char* sdp_last_error(void);   /* 이 스레드의 마지막 오류문 — 해제하지 않음 */
void        sdp_free(char* p);

#ifdef __cplusplus
}
#endif

#endif /* STRUCT_DOCPARSE_H */
