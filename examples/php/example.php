<?php
//------------------------------------------------------------------
// struct_docparse PHP 예제 — 문서마다 형식 · 라벨별 블록 수 · 앞 블록 3개
//   php -d extension=ffi example.php 문서1 [문서2 ...]
//   라이브러리 경로: 환경변수 SDP_LIB → 묶음 lib/ → 저장소 target/release
//------------------------------------------------------------------
require __DIR__ . '/../../bindings/php/StructDocParse.php';

$lib = PHP_OS_FAMILY === 'Windows' ? 'struct_docparse.dll' : 'libstruct_docparse.so';
$path = null;
foreach ([getenv('SDP_LIB') ?: '', __DIR__ . "/../../lib/$lib", __DIR__ . "/../../target/release/$lib"] as $c) {
    if ($c !== '' && is_file($c)) {
        $path = realpath($c);
        break;
    }
}
if ($path === null) {
    fwrite(STDERR, "라이브러리를 찾지 못했습니다 — SDP_LIB 에 $lib 경로를 주세요\n");
    exit(2);
}

// 문서 하나 파싱해 요약 한 덩어리 출력
function show(StructDocParse $sdp, string $file, array $opts = []): void
{
    $name = basename($file);
    try {
        $d = $sdp->parse($file, $opts);
        $kinds = array_count_values(array_column($d['blocks'], 'type'));
        printf("  %s : %s · 블록 %d · %s\n", $name, $d['format'], count($d['blocks']), json_encode($kinds, JSON_UNESCAPED_UNICODE));
        foreach (array_slice($d['blocks'], 0, 3) as $b) {
            printf("      [%s] %s\n", $b['type'], str_replace("\n", ' ', mb_substr($b['text'], 0, 50)));
        }
    } catch (StructDocParseException $e) {
        printf("  %s : 실패(%d) %s\n", $name, $e->getCode(), $e->getMessage());
    }
}

$sdp = StructDocParse::load($path, ['ocr' => ['enabled' => true, 'mode' => 'inline']]);
echo 'struct_docparse ', $sdp->version(), ' · OCR ', json_encode($sdp->status()['ocr']['enabled']), "\n";
$files = array_slice($argv, 1);
foreach ($files as $f) {
    show($sdp, $f);
}
// 이번 호출만 OCR 끄기 — 그림이면 -4 가 정상
if ($files) {
    echo "OCR 끔으로 첫 문서 다시:\n";
    show($sdp, $files[0], ['ocr' => ['enabled' => false]]);
}
$sdp->close();
