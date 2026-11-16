/*------------------------------------------------------------------
 * struct_docparse C 예제 — 문서마다 결과 JSON 앞부분과 블록 수를 보여 준다
 *=> 헤더(include/struct_docparse.h)만 쓰고, JSON 은 직접 해석하지 않는다(쓰는 쪽 JSON 라이브러리로 읽으면 된다).
 *
 *   Windows (x64 Native Tools 명령 프롬프트):
 *     cl /utf-8 /I ..\..\include example.c ..\..\target\release\struct_docparse.dll.lib
 *     (실행할 때 struct_docparse.dll · pdfium.dll · onnxruntime.dll 이 example.exe 옆이나 PATH 에)
 *   Linux:
 *     gcc -I ../../include example.c -L ../../lib -lstruct_docparse -Wl,-rpath,'$ORIGIN/../../lib' -o example
 *
 *   ./example 문서1 [문서2 ...]
 *------------------------------------------------------------------*/
#include <stdio.h>
#include <string.h>
#include "struct_docparse.h"

#ifdef _WIN32
#include <windows.h>
#endif

/*------------------------------------------------------------------
 * 로그 콜백 — 라이브러리 경고를 표준 오류로
 * -in: level = 1 경고 · 2 정보 / msg = 글 / user = 넘긴 포인터(안 씀)
 * -out: 없음
 *------------------------------------------------------------------*/
static void on_log(int32_t level, const char* msg, void* user)
{
    (void)user;
    fprintf(stderr, "[sdp %d] %s\n", level, msg);
}

/*------------------------------------------------------------------
 * JSON 글에서 "type": 나온 횟수 = 블록 수(간단히 세기)
 * -in: json = 결과 JSON
 * -out: 블록 수
 *------------------------------------------------------------------*/
static int count_blocks(const char* json)
{
    int n = 0;
    const char* p = json;
    while ((p = strstr(p, "\"type\":")) != NULL) {
        n++;
        p += 7;
    }
    return n;
}

/*------------------------------------------------------------------
 * 문서 하나 파싱해 요약 출력
 * -in: h = 핸들 / path = 문서 / call_opts = 호출 옵션 JSON(NULL 가능)
 * -out: 없음 (실패하면 코드와 오류문을 찍는다)
 *------------------------------------------------------------------*/
static void show(SdpHandle* h, const char* path, const char* call_opts)
{
    char* out = NULL;
    int32_t rc = sdp_parse(h, path, call_opts, &out);
    if (rc != SDP_OK) {
        printf("  %s : 실패(%d) %s\n", path, rc, sdp_last_error());
        return;
    }
    /* 앞 160바이트만 — UTF-8 글자 중간에서 끊길 수 있지만 예제라 그대로 둔다 */
    printf("  %s : 블록 %d · %.160s...\n", path, count_blocks(out), out);
    sdp_free(out);   /* 받은 문자열은 반드시 sdp_free */
}

#ifdef _WIN32
#include <shellapi.h>
#include <stdlib.h>
/*------------------------------------------------------------------
 * Windows: 명령줄 인자를 UTF-8 로 다시 얻기
 *=> Windows 의 C argv 는 시스템 코드 페이지(한국어 Windows 는 CP949)라 한글 경로를 그대로 넘기면
 *   라이브러리가 "UTF-8 이 아닙니다"(-1)로 거절한다. 와이드 문자 인자를 UTF-8 로 바꿔 쓴다.
 * -in: argc = 인자 수(바뀐 값을 돌려준다)
 * -out: UTF-8 인자 배열(프로그램 끝까지 쓰므로 해제하지 않는다)
 *------------------------------------------------------------------*/
static char** utf8_argv(int* argc)
{
    LPWSTR* w = CommandLineToArgvW(GetCommandLineW(), argc);
    char** a = (char**)calloc((size_t)*argc + 1, sizeof(char*));
    int i;
    for (i = 0; i < *argc; i++) {
        int n = WideCharToMultiByte(CP_UTF8, 0, w[i], -1, NULL, 0, NULL, NULL);
        a[i] = (char*)malloc((size_t)n);
        WideCharToMultiByte(CP_UTF8, 0, w[i], -1, a[i], n, NULL, NULL);
    }
    LocalFree(w);
    return a;
}
#endif

int main(int argc, char** argv)
{
    SdpHandle* h = NULL;
    int i;
#ifdef _WIN32
    SetConsoleOutputCP(CP_UTF8);   /* 한글이 깨지지 않게 */
    argv = utf8_argv(&argc);       /* 경로를 UTF-8 로 — 라이브러리는 UTF-8 만 받는다 */
#endif
    if (sdp_abi_version() != 1) {
        fprintf(stderr, "ABI 가 다릅니다: %u\n", sdp_abi_version());
        return 1;
    }
    printf("struct_docparse %s\n", sdp_version());
    if (sdp_open("{\"ocr\":{\"enabled\":true,\"mode\":\"inline\"}}", on_log, NULL, &h) != SDP_OK) {
        fprintf(stderr, "열기 실패: %s\n", sdp_last_error());
        return 1;
    }
    for (i = 1; i < argc; i++) {
        show(h, argv[i], NULL);
    }
    /* 이번 호출만 OCR 끄기 — 그림이면 -4(SDP_EUNSUPPORTED) 가 정상 */
    if (argc > 1) {
        printf("OCR 끔으로 첫 문서 다시:\n");
        show(h, argv[1], "{\"ocr\":{\"enabled\":false}}");
    }
    sdp_close(h);
    return 0;
}
