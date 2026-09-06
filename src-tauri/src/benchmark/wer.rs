#[derive(Debug, Clone, PartialEq)]
pub struct EditCounts {
    pub hits: usize,
    pub substitutions: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub total_ref: usize,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WerResult {
    pub wer: f64,
    pub words: EditCounts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CerResult {
    pub cer: f64,
    pub chars: EditCounts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvalSample {
    pub id: String,
    pub reference: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorpusEvalResult {
    pub average_wer: f64,
    pub average_cer: f64,
    pub total_words: usize,
    pub total_errors: usize,
    pub sample_results: Vec<WerResult>,
}

pub fn normalize_for_eval(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            clean.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() {
            clean.push(' ');
        }
    }
    clean.split_whitespace().collect::<Vec<&str>>().join(" ")
}

pub fn calculate_wer(reference: &str, hypothesis: &str) -> WerResult {
    let norm_ref = normalize_for_eval(reference);
    let norm_hyp = normalize_for_eval(hypothesis);

    let ref_words: Vec<&str> = norm_ref.split_whitespace().collect();
    let hyp_words: Vec<&str> = norm_hyp.split_whitespace().collect();

    let counts = calculate_edit_distance(&ref_words, &hyp_words);
    WerResult {
        wer: counts.rate,
        words: counts,
    }
}

pub fn calculate_cer(reference: &str, hypothesis: &str) -> CerResult {
    let norm_ref = normalize_for_eval(reference);
    let norm_hyp = normalize_for_eval(hypothesis);

    let ref_chars: Vec<char> = norm_ref.chars().filter(|c| !c.is_whitespace()).collect();
    let hyp_chars: Vec<char> = norm_hyp.chars().filter(|c| !c.is_whitespace()).collect();

    let counts = calculate_edit_distance(&ref_chars, &hyp_chars);
    CerResult {
        cer: counts.rate,
        chars: counts,
    }
}

fn calculate_edit_distance<T: PartialEq>(reference: &[T], hypothesis: &[T]) -> EditCounts {
    let r_len = reference.len();
    let h_len = hypothesis.len();

    if r_len == 0 {
        return EditCounts {
            hits: 0,
            substitutions: 0,
            insertions: h_len,
            deletions: 0,
            total_ref: 0,
            rate: if h_len == 0 { 0.0 } else { 1.0 },
        };
    }

    if h_len == 0 {
        return EditCounts {
            hits: 0,
            substitutions: 0,
            insertions: 0,
            deletions: r_len,
            total_ref: r_len,
            rate: 1.0,
        };
    }

    let mut dp = vec![vec![(0usize, 0usize, 0usize, 0usize); h_len + 1]; r_len + 1];

    for (i, row) in dp.iter_mut().enumerate().take(r_len + 1).skip(1) {
        row[0] = (i, 0, 0, i);
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(h_len + 1).skip(1) {
        *cell = (j, 0, j, 0);
    }

    for i in 1..=r_len {
        for j in 1..=h_len {
            if reference[i - 1] == hypothesis[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            } else {
                let sub_cost = dp[i - 1][j - 1].0 + 1;
                let ins_cost = dp[i][j - 1].0 + 1;
                let del_cost = dp[i - 1][j].0 + 1;

                if sub_cost <= ins_cost && sub_cost <= del_cost {
                    let prev = dp[i - 1][j - 1];
                    dp[i][j] = (sub_cost, prev.1 + 1, prev.2, prev.3);
                } else if ins_cost <= del_cost {
                    let prev = dp[i][j - 1];
                    dp[i][j] = (ins_cost, prev.1, prev.2 + 1, prev.3);
                } else {
                    let prev = dp[i - 1][j];
                    dp[i][j] = (del_cost, prev.1, prev.2, prev.3 + 1);
                }
            }
        }
    }

    let final_cell = dp[r_len][h_len];
    let total_errors = final_cell.1 + final_cell.2 + final_cell.3;
    let hits = r_len.saturating_sub(final_cell.1 + final_cell.3);
    let rate = (total_errors as f64) / (r_len as f64);

    EditCounts {
        hits,
        substitutions: final_cell.1,
        insertions: final_cell.2,
        deletions: final_cell.3,
        total_ref: r_len,
        rate,
    }
}

pub fn evaluate_corpus(samples: &[EvalSample], hypotheses: &[&str]) -> CorpusEvalResult {
    assert_eq!(
        samples.len(),
        hypotheses.len(),
        "Sample count must match hypothesis count"
    );

    let mut total_errors = 0;
    let mut total_words = 0;
    let mut total_cer_errors = 0;
    let mut total_chars = 0;
    let mut sample_results = Vec::with_capacity(samples.len());

    for (sample, &hyp) in samples.iter().zip(hypotheses.iter()) {
        let wer_res = calculate_wer(&sample.reference, hyp);
        let cer_res = calculate_cer(&sample.reference, hyp);

        total_errors +=
            wer_res.words.substitutions + wer_res.words.insertions + wer_res.words.deletions;
        total_words += wer_res.words.total_ref;

        total_cer_errors +=
            cer_res.chars.substitutions + cer_res.chars.insertions + cer_res.chars.deletions;
        total_chars += cer_res.chars.total_ref;

        sample_results.push(wer_res);
    }

    let average_wer = if total_words > 0 {
        (total_errors as f64) / (total_words as f64)
    } else {
        0.0
    };

    let average_cer = if total_chars > 0 {
        (total_cer_errors as f64) / (total_chars as f64)
    } else {
        0.0
    };

    CorpusEvalResult {
        average_wer,
        average_cer,
        total_words,
        total_errors,
        sample_results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_match_has_zero_wer_and_cer() {
        let ref_text = "The quick brown fox jumps over the lazy dog.";
        let hyp_text = "the quick brown fox jumps over the lazy dog";

        let wer = calculate_wer(ref_text, hyp_text);
        assert_eq!(wer.wer, 0.0);
        assert_eq!(wer.words.hits, 9);
        assert_eq!(wer.words.substitutions, 0);

        let cer = calculate_cer(ref_text, hyp_text);
        assert_eq!(cer.cer, 0.0);
    }

    #[test]
    fn calculates_substitutions_insertions_deletions() {
        let ref_text = "apple banana cherry";
        let hyp_text = "apple orange cherry extra";

        let wer = calculate_wer(ref_text, hyp_text);
        // 1 substitution (banana -> orange), 1 insertion (extra), 3 ref words -> 2 / 3 = 0.6666...
        assert_eq!(wer.words.substitutions, 1);
        assert_eq!(wer.words.insertions, 1);
        assert_eq!(wer.words.deletions, 0);
        assert!((wer.wer - 2.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn evaluate_corpus_aggregates_metrics() {
        let samples = vec![
            EvalSample {
                id: "s1".into(),
                reference: "start microphone recording".into(),
                category: "commands".into(),
            },
            EvalSample {
                id: "s2".into(),
                reference: "this is a test sentence".into(),
                category: "prose".into(),
            },
        ];
        let hypotheses = vec!["start microphone recording", "this is test sentence"]; // 1 deletion in s2

        let result = evaluate_corpus(&samples, &hypotheses);
        assert_eq!(result.total_words, 8);
        assert_eq!(result.total_errors, 1);
        assert_eq!(result.average_wer, 1.0 / 8.0);
    }
}
