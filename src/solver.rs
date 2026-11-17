use rusqlite::Connection;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct QuestionItem {
    pub id: i32,
    pub question: String,
    pub _clean_question: String,
    pub normalized_question: String,
    pub options: Vec<String>,
    pub normalized_options: Vec<String>,
    pub correct_option: i32,
    pub is_critical: bool,
    pub is_trap: bool,
    pub trap_reason: String,
    pub explanation: String,
    pub license_class: Option<String>,
    pub speed: Option<i32>,
    pub age: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct SolveResult {
    pub matched: bool,
    pub id: i32,
    pub correct_option: i32,
    pub option_text: String,
    pub explanation: String,
    pub is_critical: bool,
    pub is_trap: bool,
    pub trap_reason: String,
    pub confidence: f32,
    pub question_title: String,
}

impl Default for SolveResult {
    fn default() -> Self {
        Self {
            matched: false,
            id: 0,
            correct_option: 0,
            option_text: String::new(),
            explanation: String::new(),
            is_critical: false,
            is_trap: false,
            trap_reason: String::new(),
            confidence: 0.0,
            question_title: String::new(),
        }
    }
}

pub struct SolverEngine {
    questions: Vec<QuestionItem>,
}

fn log_debug(msg: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("noryxdrive_debug.log")
    {
        let _ = writeln!(file, "{}", msg);
    }
}

impl SolverEngine {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, question, clean_question, normalized_question, options_json, \
                    correct_option, is_critical, is_trap, trap_reason, explanation, \
                    license_class, speed, age \
             FROM questions ORDER BY LENGTH(normalized_question) DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            let opts_json: String = row.get(4).unwrap_or_default();
            let options: Vec<String> = serde_json::from_str(&opts_json).unwrap_or_default();
            let normalized_options: Vec<String> = options.iter().map(|o| normalize_text(o)).collect();
            Ok(QuestionItem {
                id: row.get(0)?,
                question: row.get(1)?,
                _clean_question: row.get(2)?,
                normalized_question: row.get(3)?,
                options,
                normalized_options,
                correct_option: row.get(5)?,
                is_critical: row.get::<_, i32>(6)? != 0,
                is_trap: row.get::<_, i32>(7)? != 0,
                trap_reason: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                explanation: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                license_class: row.get::<_, Option<String>>(10)?,
                speed: row.get::<_, Option<i32>>(11)?,
                age: row.get::<_, Option<i32>>(12)?,
            })
        })?;

        let mut questions = Vec::with_capacity(600);
        for r in rows {
            questions.push(r?);
        }

        Ok(Self { questions })
    }

    pub fn solve(&self, raw_ocr_text: &str) -> SolveResult {
        if self.questions.is_empty() || raw_ocr_text.trim().is_empty() {
            log_debug("[OCR] Text đầu vào rỗng");
            return SolveResult::default();
        }

        let norm_ocr = normalize_text(raw_ocr_text);
        log_debug(&format!("[OCR Raw {} chars]: {}", raw_ocr_text.len(), &raw_ocr_text[..raw_ocr_text.len().min(180)]));
        log_debug(&format!("[OCR Norm]: {}", &norm_ocr[..norm_ocr.len().min(180)]));

        if norm_ocr.len() < 5 {
            return SolveResult::default();
        }

        // 1. QUY TẮC LUẬT TUYỆT ĐỐI: CÂU #39 (Biển tạm thời > Biển cố định)
        if norm_ocr.contains("tam thoi") && norm_ocr.contains("co dinh") {
            if let Some(q39) = self.questions.iter().find(|q| q.id == 39) {
                let opt_text = q39
                    .options
                    .get((q39.correct_option - 1) as usize)
                    .cloned()
                    .unwrap_or_default();
                log_debug("[Solve] Khớp tuyệt đối Câu #39 (Biển tạm thời)");
                return SolveResult {
                    matched: true,
                    id: 39,
                    correct_option: q39.correct_option,
                    option_text: opt_text,
                    explanation: q39.explanation.clone(),
                    is_critical: q39.is_critical,
                    is_trap: q39.is_trap,
                    trap_reason: q39.trap_reason.clone(),
                    confidence: 100.0,
                    question_title: q39.question.clone(),
                };
            }
        }

        // 2. QUY TẮC LUẬT TUYỆT ĐỐI: Hạng C (#129) vs Hạng C1 (#128)
        if norm_ocr.contains("hang c1 duoc") || norm_ocr.contains("giay phep lai xe hang c1") {
            if let Some(q128) = self.questions.iter().find(|q| q.id == 128) {
                let opt_text = q128
                    .options
                    .get((q128.correct_option - 1) as usize)
                    .cloned()
                    .unwrap_or_default();
                log_debug("[Solve] Khớp tuyệt đối Câu #128 (Hạng C1)");
                return SolveResult {
                    matched: true,
                    id: 128,
                    correct_option: q128.correct_option,
                    option_text: opt_text,
                    explanation: q128.explanation.clone(),
                    is_critical: q128.is_critical,
                    is_trap: q128.is_trap,
                    trap_reason: q128.trap_reason.clone(),
                    confidence: 100.0,
                    question_title: q128.question.clone(),
                };
            }
        } else if norm_ocr.contains("hang c duoc") || norm_ocr.contains("giay phep lai xe hang c") {
            if let Some(q129) = self.questions.iter().find(|q| q.id == 129) {
                let opt_text = q129
                    .options
                    .get((q129.correct_option - 1) as usize)
                    .cloned()
                    .unwrap_or_default();
                log_debug("[Solve] Khớp tuyệt đối Câu #129 (Hạng C)");
                return SolveResult {
                    matched: true,
                    id: 129,
                    correct_option: q129.correct_option,
                    option_text: opt_text,
                    explanation: q129.explanation.clone(),
                    is_critical: q129.is_critical,
                    is_trap: q129.is_trap,
                    trap_reason: q129.trap_reason.clone(),
                    confidence: 100.0,
                    question_title: q129.question.clone(),
                };
            }
        }

        // 3. So khớp trực tiếp chuỗi câu hỏi chuẩn (Exact Substring Match)
        for item in &self.questions {
            if item.normalized_question.len() > 15 && norm_ocr.contains(&item.normalized_question) {
                let opt_text = item
                    .options
                    .get((item.correct_option - 1) as usize)
                    .cloned()
                    .unwrap_or_default();
                log_debug(&format!("[Solve] Khớp Substring chuẩn Câu #{}", item.id));
                return SolveResult {
                    matched: true,
                    id: item.id,
                    correct_option: item.correct_option,
                    option_text: opt_text,
                    explanation: item.explanation.clone(),
                    is_critical: item.is_critical,
                    is_trap: item.is_trap,
                    trap_reason: item.trap_reason.clone(),
                    confidence: 100.0,
                    question_title: item.question.clone(),
                };
            }
        }

        // 4. Thuật toán Độ phủ Từ vựng (Vocabulary Coverage) & Khớp Phương án Đáp án
        let ocr_words: Vec<&str> = norm_ocr.split_whitespace().collect();
        let ocr_set: HashSet<&str> = ocr_words.iter().copied().collect();

        let mut best_score = 0.0f32;
        let mut best_item: Option<&QuestionItem> = None;

        for item in &self.questions {
            let item_norm = &item.normalized_question;
            let q_words: Vec<&str> = item_norm
                .split_whitespace()
                .filter(|w| w.len() > 2)
                .collect();
            if q_words.is_empty() {
                continue;
            }

            // Tính tỷ lệ từ câu hỏi xuất hiện trên màn hình (Recall)
            let matched_q = q_words.iter().filter(|&&w| ocr_set.contains(w)).count();
            let q_coverage = (matched_q as f32) / (q_words.len() as f32);

            if q_coverage < 0.50 {
                continue;
            }

            // Tính điểm khớp các phương án đáp án (Options Matching)
            let mut opt_bonus = 0.0f32;
            if !item.normalized_options.is_empty() {
                let mut matched_opts = 0;
                for opt_norm in &item.normalized_options {
                    if opt_norm.len() > 3 && norm_ocr.contains(opt_norm) {
                        matched_opts += 1;
                    } else {
                        let opt_words: Vec<&str> = opt_norm
                            .split_whitespace()
                            .filter(|w| w.len() > 2)
                            .collect();
                        if !opt_words.is_empty() {
                            let m = opt_words.iter().filter(|&&w| ocr_set.contains(w)).count();
                            if (m as f32) / (opt_words.len() as f32) >= 0.65 {
                                matched_opts += 1;
                            }
                        }
                    }
                }
                opt_bonus = (matched_opts as f32 / item.normalized_options.len() as f32) * 40.0;
            }

            let total_score = q_coverage * 60.0 + opt_bonus;
            if total_score > best_score {
                best_score = total_score;
                best_item = Some(item);
            }
        }

        if let Some(item) = best_item {
            log_debug(&format!("[Solve] Điểm cao nhất: Câu #{} (Score: {:.1})", item.id, best_score));
            if best_score >= 35.0 {
                let opt_text = item
                    .options
                    .get((item.correct_option - 1) as usize)
                    .cloned()
                    .unwrap_or_default();

                return SolveResult {
                    matched: true,
                    id: item.id,
                    correct_option: item.correct_option,
                    option_text: opt_text,
                    explanation: item.explanation.clone(),
                    is_critical: item.is_critical,
                    is_trap: item.is_trap,
                    trap_reason: item.trap_reason.clone(),
                    confidence: best_score.min(100.0),
                    question_title: item.question.clone(),
                };
            }
        }

        log_debug("[Solve] Không vượt qua ngưỡng điểm tin cậy");
        SolveResult::default()
    }
}

pub fn remove_accents(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let base = match c {
            // A variants (including Germanic / Nordic ä, å, ā)
            'à' | 'á' | 'ả' | 'ã' | 'ạ' | 'ă' | 'ằ' | 'ắ' | 'ẳ' | 'ẵ' | 'ặ' | 'â' | 'ầ'
            | 'ấ' | 'ẩ' | 'ẫ' | 'ậ' | 'ä' | 'å' | 'ā' | 'ǎ' | 'ª' => 'a',
            'À' | 'Á' | 'Ả' | 'Ã' | 'Ạ' | 'Ă' | 'Ằ' | 'Ắ' | 'Ẳ' | 'Ẵ' | 'Ặ' | 'Â' | 'Ầ'
            | 'Ấ' | 'Ẩ' | 'Ẫ' | 'Ậ' | 'Ä' | 'Å' | 'Ā' | 'Ǎ' => 'a',

            // E variants
            'è' | 'é' | 'ẻ' | 'ẽ' | 'ẹ' | 'ê' | 'ề' | 'ế' | 'ể' | 'ễ' | 'ệ' | 'ë' | 'ē'
            | 'ě' => 'e',
            'È' | 'É' | 'Ẻ' | 'Ẽ' | 'Ẹ' | 'Ê' | 'Ề' | 'Ế' | 'Ể' | 'Ễ' | 'Ệ' | 'Ë' | 'Ē'
            | 'Ě' => 'e',

            // I variants
            'ì' | 'í' | 'ỉ' | 'ĩ' | 'ị' | 'ï' | 'ī' | 'ǐ' => 'i',
            'I' | 'Ì' | 'Í' | 'Ỉ' | 'Ĩ' | 'Ị' | 'Ï' | 'Ī' | 'Ǐ' => 'i',

            // O variants (including Germanic / Nordic ö, ø, ō)
            'ò' | 'ó' | 'ỏ' | 'õ' | 'ọ' | 'ô' | 'ồ' | 'ố' | 'ổ' | 'ỗ' | 'ộ' | 'ơ' | 'ờ'
            | 'ớ' | 'ở' | 'ỡ' | 'ợ' | 'ö' | 'ø' | 'ō' | 'ő' | 'ǒ' | 'º' => 'o',
            'O' | 'Ò' | 'Ó' | 'Ỏ' | 'Õ' | 'Ọ' | 'Ô' | 'Ồ' | 'Ố' | 'Ổ' | 'Ỗ' | 'Ộ' | 'Ơ'
            | 'Ờ' | 'Ớ' | 'Ở' | 'Ỡ' | 'Ợ' | 'Ö' | 'Ø' | 'Ō' | 'Ő' | 'Ǒ' => 'o',

            // U variants (including Germanic ü, ū)
            'ù' | 'ú' | 'ủ' | 'ũ' | 'ụ' | 'ư' | 'ừ' | 'ứ' | 'ử' | 'ữ' | 'ự' | 'ü' | 'ū'
            | 'ű' | 'ǔ' => 'u',
            'U' | 'Ù' | 'Ú' | 'Ủ' | 'Ũ' | 'Ụ' | 'Ư' | 'Ừ' | 'Ứ' | 'Ử' | 'Ữ' | 'Ự' | 'Ü'
            | 'Ū' | 'Ű' | 'Ǔ' => 'u',

            // Y variants
            'ỳ' | 'ý' | 'ỷ' | 'ỹ' | 'ỵ' | 'ÿ' => 'y',
            'Y' | 'Ỳ' | 'Ý' | 'Ỷ' | 'Ỹ' | 'Ỵ' | 'Ÿ' => 'y',

            // D / Eth variants
            'đ' | 'Đ' | 'ð' | 'Ð' => 'd',

            _ => c.to_ascii_lowercase(),
        };
        out.push(base);
    }
    out
}

pub fn normalize_text(s: &str) -> String {
    let no_acc = remove_accents(s);
    let mut out = String::with_capacity(no_acc.len());
    let mut prev_space = false;
    for c in no_acc.chars() {
        if c.is_alphanumeric() {
            out.push(c);
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }

    // Sửa các lỗi nhận diện quang học thường gặp của Windows Media OCR
    let words: Vec<&str> = out.split_whitespace().collect();
    let mut fixed = Vec::with_capacity(words.len());
    for w in words {
        match w {
            "duqc" => fixed.push("duoc".to_string()),
            "dibu" => fixed.push("dieu".to_string()),
            "c6" => fixed.push("co".to_string()),
            "khicn" => fixed.push("khien".to_string()),
            "phucng" => fixed.push("phuong".to_string()),
            _ => {
                let mut word_str = w.to_string();
                if word_str.contains("kh6i") {
                    word_str = word_str.replace("kh6i", "khoi");
                }
                if word_str.contains("luqng") {
                    word_str = word_str.replace("luqng", "luong");
                }
                fixed.push(word_str);
            }
        }
    }

    fixed.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_question_39_priority() {
        let engine = SolverEngine::new("../noryxdrive.db").expect("Database must load");
        let query = "Khi ở một vị trí vừa có biển báo hiệu đặt cố định vừa có biển báo hiệu tạm thời mà hai biển có ý nghĩa khác nhau, người tham gia giao thông đường bộ phải chấp hành hiệu lệnh của biển báo hiệu nào?";
        let res = engine.solve(query);
        assert!(res.matched);
        assert_eq!(res.id, 39);
        assert_eq!(res.correct_option, 2);
    }

    #[test]
    fn test_question_129_license_c() {
        let engine = SolverEngine::new("../noryxdrive.db").expect("Database must load");
        let query = "Người có Giấy phép lái xe hạng C được điều khiển loại xe nào dưới đây? 1-Xe ô tô tải và ô tô chuyên dùng có khối lượng toàn bộ theo thiết kế trên 3.500 kg đến 7.500 kg... 2-Xe ô tô tải và ô tô chuyên dùng có khối lượng toàn bộ theo thiết kế trên 7.500 kg... 3-Cả hai ý trên.";
        let res = engine.solve(query);
        assert!(res.matched);
        assert_eq!(res.id, 129);
        assert_eq!(res.correct_option, 3);
    }

    #[test]
    fn test_question_128_license_c1() {
        let engine = SolverEngine::new("../noryxdrive.db").expect("Database must load");
        let query = "Người có Giấy phép lái xe hạng C1 được điều khiển loại xe nào dưới đây? 1-Xe ô tô tải... 2-Xe ô tô tải và ô tô chuyên dùng có khối lượng toàn bộ theo thiết kế trên 3.500 kg đến 7.500 kg... 3-Cả hai ý trên.";
        let res = engine.solve(query);
        assert!(res.matched);
        assert_eq!(res.id, 128);
        assert_eq!(res.correct_option, 2);
    }
}
