//------------------------------------------------------------------
// 길이 재기 (SimpleRAG tok.rs 의 청킹에 쓰는 부분)
//=> 청킹은 '글 길이'와 '창으로 자르기(id 목록 → 다시 글)' 두 가지만 쓴다. 이것을 Measure 로 묶어
//   두 가지를 끼울 수 있게 한다.
//    · HfTok       — e5 tokenizer.json (HF tokenizers — SimpleRAG 와 같은 버전 · 같은 기능이라 같은 토큰)
//    · CharMeasure — 글자 수(토크나이저가 없을 때). id = 글자 코드, 되돌리기 = 글자 잇기
//------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;

use tokenizers::Tokenizer;

//------------------------------------------------------------------
// 길이 재기 약속
//=> ids · decode 는 고정 크기 창 자르기(chunk_ids)에, tlen 은 예산 비교에 쓴다.
//------------------------------------------------------------------
pub trait Measure: Send + Sync {
    // 글 → id 목록(특수 토큰 없이)
    fn ids(&self, text: &str) -> Vec<u32>;
    // id 목록 → 글(특수 토큰은 건너뜀)
    fn decode(&self, ids: &[u32]) -> String;
    // 글 길이(토큰 수 또는 글자 수)
    fn tlen(&self, s: &str) -> usize;
}

//------------------------------------------------------------------
// HF 토크나이저 + 길이 캐시
// -필드: inner = HF 토크나이저(잘라내기 · 채우기 끔)
// -필드: cache = 글자 → 토큰 수 (2만 개를 넘으면 비운다 — SimpleRAG 와 같음)
//------------------------------------------------------------------
pub struct HfTok {
    pub inner: Tokenizer,
    cache: Mutex<HashMap<String, usize>>,
}

impl HfTok {
    //------------------------------------------------------------------
    // tokenizer.json 읽기
    //=> 잘라내기 · 채우기는 끈다 — 긴 글도 전부 세야 예산 비교가 맞다(Python no_truncation)
    //
    // -in: path = tokenizer.json 경로
    //
    // -out: Ok(HfTok)
    // -out: error = 파일 없음 · 모양 틀림이면 Err("tokenizer 로딩 실패: …")
    //------------------------------------------------------------------
    pub fn from_file(path: &str) -> Result<HfTok, String> {
        let mut t = Tokenizer::from_file(path).map_err(|e| format!("tokenizer 로딩 실패: {}", e))?;
        let _ = t.with_truncation(None);
        t.with_padding(None);
        Ok(HfTok { inner: t, cache: Mutex::new(HashMap::new()) })
    }
}

impl Measure for HfTok {
    // 토큰 id (특수 토큰 없이)
    fn ids(&self, text: &str) -> Vec<u32> {
        self.inner.encode(text, false).map(|e| e.get_ids().to_vec()).unwrap_or_default()
    }

    // id → 글자 (특수 토큰은 건너뜀 — Python tok.decode 기본값)
    fn decode(&self, ids: &[u32]) -> String {
        self.inner.decode(ids, true).unwrap_or_default()
    }

    // 토큰 수 — 같은 글을 자주 다시 재므로 캐시
    fn tlen(&self, s: &str) -> usize {
        if let Some(n) = self.cache.lock().unwrap().get(s) {
            return *n;
        }
        let n = self.ids(s).len();
        let mut c = self.cache.lock().unwrap();
        // 캐시가 끝없이 커지지 않게 — 넘치면 통째로 비운다
        if c.len() > 20000 {
            c.clear();
        }
        c.insert(s.to_string(), n);
        n
    }
}

//------------------------------------------------------------------
// 글자 수로 재기 (토크나이저 없을 때)
//=> 모든 크기 옵션(tokens · overlap · hard_max …)이 글자 수 뜻이 된다. 공백도 한 글자로 센다.
//------------------------------------------------------------------
pub struct CharMeasure;

impl Measure for CharMeasure {
    // 글자 하나 = id 하나(유니코드 코드값)
    fn ids(&self, text: &str) -> Vec<u32> {
        text.chars().map(|c| c as u32).collect()
    }

    // 코드값 → 글자 (잘못된 값은 건너뜀)
    fn decode(&self, ids: &[u32]) -> String {
        ids.iter().filter_map(|&i| char::from_u32(i)).collect()
    }

    // 글자 수
    fn tlen(&self, s: &str) -> usize {
        s.chars().count()
    }
}
