<?php
//------------------------------------------------------------------
// struct_docparse PHP 바인딩 (PHP 7.4+ FFI 확장)
//=> php.ini 에 extension=ffi, CLI 가 아니면 ffi.enable=true 가 필요하다.
//   $sdp = StructDocParse::load('/opt/sdp/lib/libstruct_docparse.so', ['ocr' => ['mode' => 'inline']]);
//   $doc = $sdp->parse('보고서.hwpx');            // 연관 배열
//------------------------------------------------------------------

// 라이브러리가 돌려준 오류 — getCode() 가 상태 코드(-1 … -99)
class StructDocParseException extends RuntimeException
{
}

final class StructDocParse
{
    // 헤더(include/struct_docparse.h)에서 쓰는 함수만 옮긴 선언
    private const CDEF = <<<'C'
typedef struct SdpHandle SdpHandle;
uint32_t sdp_abi_version(void);
const char* sdp_version(void);
int32_t sdp_open(const char* options_json, void* log, void* user, SdpHandle** out);
void sdp_close(SdpHandle* h);
int32_t sdp_set_option(SdpHandle* h, const char* key, const char* value_json);
int32_t sdp_get_status(SdpHandle* h, char** out_json);
int32_t sdp_parse(SdpHandle* h, const char* path, const char* call_opts_json, char** out_json);
int32_t sdp_extract_text(SdpHandle* h, const char* path, int32_t fix_tables, char** out_json);
int32_t sdp_pdf_info(SdpHandle* h, const char* path, char** out_json);
const char* sdp_last_error(void);
void sdp_free(char* p);
C;

    private FFI $ffi;
    private $h;
    private string $libPath;

    // 열린 FFI 와 핸들을 쥔다
    private function __construct(FFI $ffi, $h, string $libPath)
    {
        $this->ffi = $ffi;
        $this->h = $h;
        $this->libPath = $libPath;
    }

    //------------------------------------------------------------------
    // 라이브러리 열기 + sdp_open
    //
    // -in: $libPath = struct_docparse.dll / libstruct_docparse.so 경로
    // -in: $options = 열 때 옵션 배열(빈 배열이면 기본값)
    //
    // -out: StructDocParse
    // -out: error = 적재 실패 FFI\Exception, ABI 다름 · 열기 실패 StructDocParseException
    //------------------------------------------------------------------
    public static function load(string $libPath, array $options = []): self
    {
        $ffi = FFI::cdef(self::CDEF, $libPath);
        if ($ffi->sdp_abi_version() !== 1) {
            throw new StructDocParseException('ABI 가 다릅니다: ' . $ffi->sdp_abi_version(), -1);
        }
        $out = $ffi->new('SdpHandle*');
        $json = $options ? json_encode($options, JSON_UNESCAPED_UNICODE) : null;
        $rc = $ffi->sdp_open($json, null, null, FFI::addr($out));
        if ($rc !== 0) {
            throw new StructDocParseException(FFI::string($ffi->sdp_last_error()), $rc);
        }
        return new self($ffi, $out, $libPath);
    }

    // 결과 받기 — 0 이면 JSON 을 배열로 읽고 sdp_free, 아니면 예외
    private function take(int $rc, $out)
    {
        if ($rc !== 0) {
            throw new StructDocParseException(FFI::string($this->ffi->sdp_last_error()), $rc);
        }
        if (FFI::isNull($out)) {
            return null;
        }
        $s = FFI::string($out);
        $this->ffi->sdp_free($out);
        return json_decode($s, true);
    }

    // 적재한 라이브러리 경로
    public function libPath(): string
    {
        return $this->libPath;
    }

    // 버전 문자열 — "0.1.0 blocks=5 ocr=…"
    public function version(): string
    {
        return FFI::string($this->ffi->sdp_version());
    }

    // 상태(OCR 켜짐 · 모델 · pdfium)
    public function status(): array
    {
        $out = $this->ffi->new('char*');
        return $this->take($this->ffi->sdp_get_status($this->h, FFI::addr($out)), $out);
    }

    //------------------------------------------------------------------
    // 문서 파싱
    //
    // -in: $path     = 파일 경로(UTF-8)
    // -in: $callOpts = 호출 옵션 배열(예: ['ocr' => ['enabled' => false]])
    //
    // -out: 결과 배열(format · title · pages · warnings · raw_text · ocr · blocks)
    // -out: error = 실패면 StructDocParseException(getCode() 로 종류 구분)
    //------------------------------------------------------------------
    public function parse(string $path, array $callOpts = []): array
    {
        $out = $this->ffi->new('char*');
        $opts = $callOpts ? json_encode($callOpts, JSON_UNESCAPED_UNICODE) : null;
        return $this->take($this->ffi->sdp_parse($this->h, $path, $opts, FFI::addr($out)), $out);
    }

    // 평문 추출 — ['text' => …, 'table_rows' => …]
    public function extractText(string $path, bool $fixTables = true): array
    {
        $out = $this->ffi->new('char*');
        return $this->take($this->ffi->sdp_extract_text($this->h, $path, $fixTables ? 1 : 0, FFI::addr($out)), $out);
    }

    // 실행 중 옵션 바꾸기 — 예: setOption('ocr.enabled', false)
    public function setOption(string $key, $value): void
    {
        $rc = $this->ffi->sdp_set_option($this->h, $key, json_encode($value));
        if ($rc !== 0) {
            throw new StructDocParseException(FFI::string($this->ffi->sdp_last_error()), $rc);
        }
    }

    // 핸들 닫기(OCR 엔진도 내린다)
    public function close(): void
    {
        if ($this->h !== null) {
            $this->ffi->sdp_close($this->h);
            $this->h = null;
        }
    }

    // 객체가 사라질 때 닫는다
    public function __destruct()
    {
        $this->close();
    }
}
