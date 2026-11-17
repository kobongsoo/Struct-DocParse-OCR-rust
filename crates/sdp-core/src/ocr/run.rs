//------------------------------------------------------------------
// 쪽 OCR 돌리기 → 캐시 저장 (SimpleRAG indexer::run_ocr_queue 의 쪽 처리 부분을 옮김)
//=> 대기열·실패 목록·진행 표시 정책은 호스트에 남고, 여기서는 "이 파일의 이 쪽들을 OCR 해 캐시에 넣어라" 만 한다.
//   PDF 는 200dpi 로 그려 OCR 하고 글자층과 겹친 줄을 뺀다. 그림 파일은 쪽(TIFF 는 여러 쪽)마다 OCR 한다.
//   A4 보다 큰 쪽 · 그림은 A4 크기로 줄여 OCR 하고, 좌표는 원래 기준(PDF 200dpi · 그림 원래 픽셀)으로 되돌려 저장한다.
//------------------------------------------------------------------

use std::time::Instant;

use super::{image, Cache, ENGINE};

//------------------------------------------------------------------
// 한 번 돌린 결과
// -필드: pages   = 이번에 새로 OCR 한 쪽 수(캐시에 있던 쪽은 세지 않음)
// -필드: skipped = 캐시에 이미 있어 건너뛴 쪽 수
// -필드: stopped = 진행 콜백이 멈추라고 해서 중간에 섰나
//------------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub pages: usize,
    pub skipped: usize,
    pub stopped: bool,
}

//------------------------------------------------------------------
// 파일의 쪽들을 OCR 해 캐시에 넣기 (핵심)
//=> 1) 내용 해시로 캐시를 읽는다(없으면 새로)
//   2) 쪽마다 — 캐시에 있으면 건너뛰고, 없으면 OCR 해 바로 캐시에 쓴다(중간에 꺼져도 한 쪽씩 남게)
//   3) 쪽 하나가 끝날 때마다 progress(몇 번째, 전체, 걸린 ms) — true 를 돌려주면 그 자리에서 선다
//
// -in: path     = PDF 또는 그림 파일
// -in: pages    = OCR 할 쪽 번호(1부터). None 이면 그림 파일은 모든 쪽, PDF 는 스캔 판정으로 고른 쪽
// -in: progress = 쪽마다 불림 (k(1부터), of, ms) → 멈출지
//
// -out: Ok(Run)
// -out: error = 읽기·그리기·추론·캐시 쓰기 실패면 Err("OCRError: …"). 이미 끝낸 쪽은 캐시에 남는다
//------------------------------------------------------------------
pub fn ocr_pages(path: &str, pages: Option<Vec<usize>>, progress: &mut dyn FnMut(usize, usize, f64) -> bool) -> Result<Run, String> {
    let mut run = Run::default();
    let hash = crate::hash::content_hash(path).map_err(|e| format!("OCRError: 읽기 실패: {}", e))?;
    let mut cache = super::load_cache(&hash).unwrap_or_else(|| Cache { engine: ENGINE.into(), ..Default::default() });
    if image::is_image(path) {
        cache.dpi = 0.0;
        let frames = image::load(path).map_err(|e| format!("OCRError: {}", e))?;
        // 고른 쪽이 없으면 모든 쪽
        let want: Vec<usize> = pages.unwrap_or_else(|| (1..=frames.len()).collect()).into_iter()
            .filter(|&n| n >= 1 && n <= frames.len()).collect();
        let of = want.len();
        for (k, &no) in want.iter().enumerate() {
            if cache.pages.contains_key(&no) {
                run.skipped += 1;
                continue;
            }
            let img = &frames[no - 1];
            let t0 = Instant::now();
            // 아이콘처럼 작은 그림은 읽지 않는다 — 빈 쪽으로 남겨 "글자 없는 그림"이 된다(줄이기 전 크기로 판단)
            // 큰 그림은 A4 크기로 줄여 읽고 좌표는 원래 픽셀로 되돌린다(ocr_image_file)
            // 누운 그림이면 바로 세운 좌표와 각도가 함께 온다
            let (lines, rot) = if img.w.min(img.h) < image::MIN_SHORT_SIDE { (vec![], 0) } else { super::ocr_image_file(img)? };
            cache.put_page(no, super::to_cached(&lines), rot);
            super::save_cache(&hash, &cache)?;
            run.pages += 1;
            if progress(k + 1, of, t0.elapsed().as_secs_f64() * 1000.0) {
                run.stopped = true;
                break;
            }
        }
        return Ok(run);
    }
    cache.dpi = super::DPI;
    // PDF — 고른 쪽이 없으면 추출과 같은 스캔 판정으로 고른다
    let want = match pages {
        Some(p) => p,
        None => pdf_scan_pages(path)?,
    };
    let data = std::fs::read(path).map_err(|e| format!("OCRError: 읽기 실패: {}", e))?;
    let of = want.len();
    for (k, &no) in want.iter().enumerate() {
        if no == 0 || cache.pages.contains_key(&no) {
            run.skipped += 1;
            continue;
        }
        let t0 = Instant::now();
        // 큰 쪽은 A4 크기가 되게 낮은 해상도로(ocr_page_cap), 작은 쪽은 최대 1.4배 높은 해상도로(ocr_page_up) 그린다
        let r = crate::pdfium::render_page_capped(&data, no - 1, super::DPI, crate::config::cfg().ocr_page_cap, crate::config::cfg().ocr_page_up)
            .map_err(|e| format!("OCRError: {}", e))?;
        let (lines, rot) = super::ocr_image(&r.img)?;
        // 글자층과 겹친 줄은 뺀다 — 그 자리에는 더 정확한 글자층 글이 있다(같은 그림 좌표끼리 비교하므로 되돌리기 전에)
        // 쪽을 돌렸으면 글자층 상자도 같은 식으로 돌려 바로 선 좌표끼리 비교한다
        let chars: Vec<[f32; 4]> = if rot == 0 {
            r.chars.clone()
        } else {
            r.chars.iter().map(|b| super::imgops::rot_bbox(*b, rot, r.img.w, r.img.h)).collect()
        };
        let lines = super::drop_overlapping(lines, &chars);
        // 캐시 좌표는 늘 200dpi 기준 — 구조 분석(docmodel/pdf.rs)이 쪽 크기를 200dpi 로 바꿔 맞춘다
        let lines = super::scale_lines(lines, super::DPI / r.dpi);
        cache.put_page(no, super::to_cached(&lines), rot);
        super::save_cache(&hash, &cache)?;
        run.pages += 1;
        if progress(k + 1, of, t0.elapsed().as_secs_f64() * 1000.0) {
            run.stopped = true;
            break;
        }
    }
    Ok(run)
}

//------------------------------------------------------------------
// PDF 의 OCR 할 쪽 고르기 — 추출(docmodel::pdf)과 같은 판정·상한
//
// -in: path = PDF 경로
//
// -out: Ok(쪽 번호 1부터)
// -out: error = PDF 열기 실패면 Err("OCRError: …")
//------------------------------------------------------------------
pub fn pdf_scan_pages(path: &str) -> Result<Vec<usize>, String> {
    let (raw, _) = crate::pdfium::read_pages_keep(path, false, false, None, |_, _| true).map_err(|e| format!("OCRError: {}", e))?;
    Ok(super::pdf_ocr_pages(&raw).0.into_iter().map(|i| i + 1).collect())
}
