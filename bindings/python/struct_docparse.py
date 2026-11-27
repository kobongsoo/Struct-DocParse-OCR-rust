# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# struct_docparse.dll 파이썬 래퍼 (ctypes) — 설계서 §15-4
#=> 측정 스크립트(bench)에서 DLL 결과를 그대로 재려고 만든 얇은 껍데기다.
#   SimpleRAG Python 판 docmodel 은 바꾸지 않는다(기존 측정과 이어지도록).
#
#   사용 예:
#     from struct_docparse import DocParser
#     with DocParser(r"...\struct_docparse.dll", {"ocr": {"mode": "inline"}}) as p:
#         doc = p.parse(r"C:\문서\보고서.hwpx")
#         for b in doc["blocks"]:
#             print(b["type"], b["text"][:40])
#------------------------------------------------------------------
import ctypes
import json
import os

_LOG_FN = ctypes.CFUNCTYPE(None, ctypes.c_int32, ctypes.c_char_p, ctypes.c_void_p)
_PROGRESS_FN = ctypes.CFUNCTYPE(ctypes.c_int32, ctypes.c_int32, ctypes.c_int32, ctypes.c_double, ctypes.c_void_p)


#------------------------------------------------------------------
# DLL 이 돌려준 오류
# -필드: code = 상태 코드(SDP_E*) / msg = 오류문
#------------------------------------------------------------------
class DocParseError(Exception):
    #------------------------------------------------------------------
    # 오류 만들기
    #=> 상태 코드와 오류문을 함께 담는다.
    #
    # -in: code = 상태 코드
    # -in: msg  = 오류문
    #
    # -out: 없음
    # -out: error = 예외 없음
    #------------------------------------------------------------------
    def __init__(self, code, msg):
        super().__init__(f"[{code}] {msg}")
        self.code = code
        self.msg = msg


#------------------------------------------------------------------
# 문서 파서 핸들
# -필드: dll = 적재한 DLL / h = sdp_open 핸들 / _log = 콜백(가비지 수집 막기용으로 쥔다)
#------------------------------------------------------------------
class DocParser:
    #------------------------------------------------------------------
    # DLL 적재 + 열기
    #=> 1) DLL 을 적재하고 함수 모양(argtypes)을 정한다
    #    2) 옵션 JSON 으로 sdp_open
    #
    # -in: dll_path = struct_docparse.dll 경로
    # -in: options  = 열 때 옵션 사전(None 이면 기본값 — OCR 켬 · inline)
    # -in: log      = 로그 받을 함수(level, msg) — None 이면 버림
    #
    # -out: 없음
    # -out: error = 적재 실패 OSError, 열기 실패 DocParseError
    #------------------------------------------------------------------
    def __init__(self, dll_path, options=None, log=None):
        # Windows 는 DLL 폴더를 검색 경로에 더한다(리눅스에는 이 함수가 없고, 라이브러리가 옆 파일을 직접 연다)
        if hasattr(os, "add_dll_directory"):
            os.add_dll_directory(os.path.dirname(os.path.abspath(dll_path)))
        self.dll = d = ctypes.CDLL(dll_path)
        cp, vp = ctypes.c_char_p, ctypes.c_void_p
        out = ctypes.POINTER(ctypes.c_void_p)
        d.sdp_abi_version.restype = ctypes.c_uint32
        d.sdp_version.restype = cp
        d.sdp_last_error.restype = cp
        d.sdp_open.argtypes = [cp, _LOG_FN, vp, ctypes.POINTER(vp)]
        d.sdp_close.argtypes = [vp]
        d.sdp_set_option.argtypes = [vp, cp, cp]
        d.sdp_get_status.argtypes = [vp, out]
        d.sdp_supports.argtypes = [vp, cp]
        d.sdp_parse.argtypes = [vp, cp, cp, out]
        d.sdp_extract_text.argtypes = [vp, cp, ctypes.c_int32, out]
        d.sdp_ocr_pages.argtypes = [vp, cp, cp, _PROGRESS_FN, vp, out]
        d.sdp_ocr_file.argtypes = [vp, cp, cp, out]
        d.sdp_ocr_unload.argtypes = [vp]
        d.sdp_ocr_cache_remove.argtypes = [vp, cp]
        d.sdp_pdf_info.argtypes = [vp, cp, out]
        d.sdp_pdf_page_texts.argtypes = [vp, cp, out]
        d.sdp_free.argtypes = [vp]
        # 청킹 다시 자르기(0.3.0~) — 옛 DLL 에는 없으므로 있을 때만 모양을 정한다
        if hasattr(d, "sdp_chunk"):
            d.sdp_chunk.argtypes = [vp, cp, cp, out]
        # 로그 콜백 — 바이트를 글로 바꿔 넘긴다
        self._log = _LOG_FN((lambda lv, m, u: log(lv, m.decode("utf-8", "replace"))) if log else (lambda lv, m, u: None))
        h = vp()
        rc = d.sdp_open(_enc(json.dumps(options or {})), self._log, None, ctypes.byref(h))
        if rc != 0:
            raise DocParseError(rc, self._err())
        self.h = h

    # with 문 — 들어갈 때 자신
    def __enter__(self):
        return self

    # with 문 — 나갈 때 닫기
    def __exit__(self, *a):
        self.close()

    #------------------------------------------------------------------
    # 닫기 — 핸들을 돌려주고 OCR 엔진도 내린다(두 번 불러도 됨)
    #
    # -in: 없음
    #
    # -out: 없음
    # -out: error = 예외 없음
    #------------------------------------------------------------------
    def close(self):
        if getattr(self, "h", None):
            self.dll.sdp_close(self.h)
            self.h = None

    # 이 스레드의 마지막 오류문
    def _err(self):
        return (self.dll.sdp_last_error() or b"").decode("utf-8", "replace")

    #------------------------------------------------------------------
    # 결과 받기 — 상태 코드가 0 이면 JSON 을 읽고 sdp_free, 아니면 예외
    #
    # -in: rc = 상태 코드 / p = 결과 포인터
    #
    # -out: 사전(또는 None)
    # -out: error = rc != 0 이면 DocParseError
    #------------------------------------------------------------------
    def _take(self, rc, p):
        if rc != 0:
            raise DocParseError(rc, self._err())
        if not p.value:
            return None
        try:
            return json.loads(ctypes.string_at(p.value).decode("utf-8"))
        finally:
            self.dll.sdp_free(p)

    # 버전 문자열
    def version(self):
        return self.dll.sdp_version().decode("utf-8")

    # 상태(OCR 켜짐 · 모델 · pdfium)
    def status(self):
        p = ctypes.c_void_p()
        return self._take(self.dll.sdp_get_status(self.h, ctypes.byref(p)), p)

    # 실행 중 옵션 바꾸기 — 예: set_option("ocr.enabled", False)
    def set_option(self, key, value):
        rc = self.dll.sdp_set_option(self.h, _enc(key), _enc(json.dumps(value)))
        if rc != 0:
            raise DocParseError(rc, self._err())

    #------------------------------------------------------------------
    # 문서 파싱 — 결과 사전(format · title · pages · warnings · raw_text · ocr · blocks)
    #
    # -in: path       = 파일 경로
    # -in: title      = 문서 제목(없으면 파일 이름)
    # -in: keep_pages = PDF 에서 읽을 쪽(0부터) — 주면 그 쪽만, OCR 없이
    # -in: ocr        = True/False 로 이번 호출만 OCR 켬/끔.
    #                   (0.4.0~) 사전이면 호출 옵션 ocr 그대로 — 예: {"force_when": "missing_digits"},
    #                   {"force_pages": [9, 12], "force_mode": "replace"}
    # -in: chunk      = (0.3.0~) True 면 청크도 — 결과에 chunk_version · chunking · chunks 가 붙는다.
    #                   사전이면 이번 호출만 덮어쓸 청킹 옵션(예: {"tokens": 256}). None 이면 청킹 안 함
    #
    # -out: 결과 사전
    # -out: error = 실패면 DocParseError (0.2.x DLL 에 chunk 를 주면, 0.3.x DLL 에 ocr 사전의 force_* 를 주면 -1 모르는 옵션)
    #------------------------------------------------------------------
    def parse(self, path, title=None, keep_pages=None, ocr=None, chunk=None):
        opts = {}
        if title is not None:
            opts["title"] = title
        if keep_pages is not None:
            opts["keep_pages"] = list(keep_pages)
        # OCR — True/False 는 켬/끔만, 사전은 그대로(강제 OCR 옵션 등)
        if isinstance(ocr, dict):
            opts["ocr"] = dict(ocr)
        elif ocr is not None:
            opts["ocr"] = {"enabled": bool(ocr)}
        # 청킹 — True/False 는 그대로, 사전은 켜고 덮어쓰기
        if chunk is not None:
            opts["chunk"] = chunk if isinstance(chunk, (bool, dict)) else bool(chunk)
        p = ctypes.c_void_p()
        return self._take(self.dll.sdp_parse(self.h, _enc(path), _enc(json.dumps(opts)), ctypes.byref(p)), p)

    #------------------------------------------------------------------
    # 저장해 둔 파싱 결과를 다시 청킹 (0.3.0~)
    #=> 파싱 · OCR 없이 크기만 바꿔 다시 자른다.
    #    1) parse() 결과 사전을 JSON 글로 넘긴다
    #    2) opts 가 있으면 열 때 청킹 기본값 위에 이번만 덮어쓴다
    #
    # -in: doc  = parse() 결과 사전
    # -in: opts = 청킹 옵션 사전(예: {"tokens": 256}) — None 이면 열 때 기본값
    #
    # -out: {"chunk_version", "chunking", "chunks"}
    # -out: error = 실패면 DocParseError, 0.2.x DLL 이면 DocParseError(-1, "sdp_chunk 없음")
    #------------------------------------------------------------------
    def chunk(self, doc, opts=None):
        # 옛 DLL 에는 함수가 없다 — 알아보기 쉽게 같은 예외로
        if not hasattr(self.dll, "sdp_chunk"):
            raise DocParseError(-1, "이 라이브러리에는 sdp_chunk 가 없습니다(0.3.0 부터)")
        p = ctypes.c_void_p()
        oj = _enc(json.dumps(opts)) if opts is not None else None
        return self._take(self.dll.sdp_chunk(self.h, _enc(json.dumps(doc, ensure_ascii=False)), oj, ctypes.byref(p)), p)

    # 평문 추출 — (본문, 표 행 수)
    def extract_text(self, path, fix_tables=True):
        p = ctypes.c_void_p()
        v = self._take(self.dll.sdp_extract_text(self.h, _enc(path), 1 if fix_tables else 0, ctypes.byref(p)), p)
        return v["text"], v["table_rows"]

    #------------------------------------------------------------------
    # 쪽 OCR → 캐시
    #
    # -in: path     = PDF · 그림 파일
    # -in: pages    = 쪽 번호(1부터) 목록, None 이면 남은 쪽 전부
    # -in: progress = (k, of, ms) → True 면 멈춤. None 이면 끝까지
    #
    # -out: {"pages", "skipped", "stopped"}
    # -out: error = 실패면 DocParseError
    #------------------------------------------------------------------
    def ocr_pages(self, path, pages=None, progress=None):
        cb = _PROGRESS_FN((lambda k, of, ms, u: 1 if progress(k, of, ms) else 0) if progress else (lambda k, of, ms, u: 0))
        p = ctypes.c_void_p()
        pj = _enc(json.dumps(list(pages))) if pages is not None else None
        return self._take(self.dll.sdp_ocr_pages(self.h, _enc(path), pj, cb, None, ctypes.byref(p)), p)

    # 파일 OCR 시험(캐시 안 씀)
    def ocr_file(self, path, pages=None):
        p = ctypes.c_void_p()
        pj = _enc(json.dumps(list(pages))) if pages is not None else None
        return self._take(self.dll.sdp_ocr_file(self.h, _enc(path), pj, ctypes.byref(p)), p)

    # PDF 속성
    def pdf_info(self, path):
        p = ctypes.c_void_p()
        return self._take(self.dll.sdp_pdf_info(self.h, _enc(path), ctypes.byref(p)), p)


# 글 → UTF-8 바이트(None 은 그대로)
def _enc(s):
    return None if s is None else s.encode("utf-8")
