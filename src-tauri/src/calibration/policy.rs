//! Model-free quality, stability, and cache admission contracts.
#[derive(Debug, Clone)]
pub struct Assessment {
    pub median_ms: f64,
    pub p95_ms: f64,
    pub wer: f64,
    pub cer: f64,
    pub character_language: bool,
    pub repeats: usize,
    pub spill: bool,
    pub labelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub path: String,
    pub bytes: u64,
    pub modified_ns: u128,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    pub hardware: String,
    pub versions: String,
    pub files: Vec<FileStamp>,
    pub language: String,
}

pub fn select_winner(rows: &[Assessment]) -> Option<usize> {
    let eligible = |row: &Assessment| {
        let error = if row.character_language {
            row.cer
        } else {
            row.wer
        };
        row.labelled
            && row.repeats >= 3
            && !row.spill
            && error.is_finite()
            && error >= 0.0
            && error <= if row.character_language { 0.03 } else { 0.05 }
            && row.median_ms.is_finite()
            && row.median_ms > 0.0
            && row.p95_ms.is_finite()
            && row.p95_ms >= row.median_ms
            && row.p95_ms <= row.median_ms * 1.35 + 30.0
    };
    let error = |row: &Assessment| {
        if row.character_language {
            row.cer
        } else {
            row.wer
        }
    };
    let best_quality = rows
        .iter()
        .filter(|row| eligible(row))
        .map(error)
        .min_by(f64::total_cmp)?;
    rows.iter()
        .enumerate()
        .filter(|(_, row)| eligible(row) && error(row) <= best_quality + 0.005)
        .min_by(|(_, a), (_, b)| a.median_ms.total_cmp(&b.median_ms))
        .map(|(index, _)| index)
}
pub fn cache_matches(saved: &Fingerprint, current: &Fingerprint) -> bool {
    saved == current
}

pub fn median_p95(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    };
    let p95 = sorted[((sorted.len() as f64 * 0.95).ceil() as usize).saturating_sub(1)];
    Some((median, p95))
}
fn edit_distance<T: PartialEq>(reference: &[T], hypothesis: &[T]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    for (i, item) in reference.iter().enumerate() {
        let mut row = vec![i + 1];
        for (j, other) in hypothesis.iter().enumerate() {
            row.push(
                (row[j] + 1)
                    .min(previous[j + 1] + 1)
                    .min(previous[j] + usize::from(item != other)),
            );
        }
        previous = row;
    }
    previous[hypothesis.len()]
}
pub fn error_rates(reference: &str, hypothesis: &str) -> Option<(f64, f64)> {
    if !reference.chars().any(char::is_alphanumeric) {
        return None;
    }
    let normalized = |text: &str| {
        static SEPARATORS: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"[^\p{L}\p{N}\p{M}+\-]+").unwrap());
        SEPARATORS
            .replace_all(&text.to_lowercase().replace('−', "-"), " ")
            .into_owned()
    };
    let a = normalized(reference);
    let b = normalized(hypothesis);
    let words_a: Vec<&str> = a.split_whitespace().collect();
    let words_b: Vec<&str> = b.split_whitespace().collect();
    let chars_a: Vec<char> = a.chars().filter(|c| !c.is_whitespace()).collect();
    let chars_b: Vec<char> = b.chars().filter(|c| !c.is_whitespace()).collect();
    if words_a.is_empty() || chars_a.is_empty() {
        return None;
    }
    Some((
        edit_distance(&words_a, &words_b) as f64 / words_a.len() as f64,
        edit_distance(&chars_a, &chars_b) as f64 / chars_a.len() as f64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn passing(ms: f64) -> Assessment {
        Assessment {
            median_ms: ms,
            p95_ms: ms,
            wer: 0.0,
            cer: 0.0,
            character_language: false,
            repeats: 3,
            spill: false,
            labelled: true,
        }
    }
    #[test]
    fn faster_but_inaccurate_candidate_cannot_win() {
        let mut inaccurate = passing(5.0);
        inaccurate.wer = 0.2;
        assert_eq!(select_winner(&[inaccurate, passing(10.0)]), Some(1));
    }
    #[test]
    fn spill_and_unstable_tail_latency_cannot_win() {
        let mut spill = passing(1.0);
        spill.spill = true;
        let mut unstable = passing(2.0);
        unstable.p95_ms = 200.0;
        assert_eq!(select_winner(&[spill, unstable, passing(100.0)]), Some(2));
    }
    #[test]
    fn unlabelled_or_under_sampled_results_cannot_be_applied() {
        let mut unlabelled = passing(1.0);
        unlabelled.labelled = false;
        let mut under_sampled = passing(2.0);
        under_sampled.repeats = 1;
        assert_eq!(select_winner(&[unlabelled, under_sampled]), None);
    }
    #[test]
    fn character_languages_use_cer_to_compare_actual_text_edits() {
        let mut candidate = passing(1.0);
        candidate.character_language = true;
        candidate.wer = 1.0;
        candidate.cer = 0.01;
        assert_eq!(select_winner(&[candidate]), Some(0));
    }
    #[test]
    fn model_runtime_and_language_changes_invalidate_cached_winners() {
        let saved = Fingerprint {
            hardware: "gpu driver cpu".into(),
            versions: "torch-v1/runtime-a".into(),
            language: "en".into(),
            files: vec![FileStamp {
                path: "weights".into(),
                bytes: 123,
                modified_ns: 45,
            }],
        };
        for changed in [
            Fingerprint {
                versions: "torch-v2/runtime-a".into(),
                ..saved.clone()
            },
            Fingerprint {
                language: "hi".into(),
                ..saved.clone()
            },
            Fingerprint {
                files: vec![FileStamp {
                    path: "weights".into(),
                    bytes: 123,
                    modified_ns: 46,
                }],
                ..saved.clone()
            },
        ] {
            assert!(!cache_matches(&saved, &changed));
        }
        assert!(cache_matches(&saved, &saved));
    }
}

#[cfg(test)]
mod admission_edge_tests {
    use super::*;

    fn assessment() -> Assessment {
        Assessment {
            median_ms: 100.0,
            p95_ms: 110.0,
            wer: 0.0,
            cer: 0.0,
            character_language: false,
            repeats: 3,
            spill: false,
            labelled: true,
        }
    }

    #[test]
    fn combining_marks_without_words_are_not_a_labelled_corpus() {
        assert!(error_rates("\u{0301}\u{0300}", "\u{0301}\u{0300}").is_none());
    }

    #[test]
    fn dropping_a_numeric_sign_is_an_accuracy_error() {
        let (wer, cer) = error_rates("Temperature is -5 degrees", "Temperature is 5 degrees")
            .expect("reference has real words");
        assert!(
            wer > 0.0 || cer > 0.0,
            "A sign changes the reference's meaning"
        );
    }

    #[test]
    fn symbol_only_reference_is_rejected_but_real_scripts_are_admitted() {
        for reference in ["", " \n\t", "...—?!", "\u{200b}\u{200d}"] {
            assert!(error_rates(reference, reference).is_none(), "{reference:?}");
        }
        for reference in ["नमस्ते दुनिया", "你好世界", "مرحبا بالعالم", "হ্যালো পৃথিবী"]
        {
            assert_eq!(error_rates(reference, reference), Some((0.0, 0.0)));
        }
    }

    #[test]
    fn empty_hypothesis_is_not_a_perfect_match() {
        assert_eq!(error_rates("hello world", ""), Some((1.0, 1.0)));
    }

    #[test]
    fn invalid_latency_statistics_never_qualify() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0] {
            let mut row = assessment();
            row.median_ms = value;
            assert_eq!(select_winner(&[row]), None);
        }
        let mut row = assessment();
        row.p95_ms = 99.0;
        assert_eq!(select_winner(&[row]), None);
    }

    #[test]
    fn fastest_candidate_must_remain_within_best_accuracy_tolerance() {
        let mut fast = assessment();
        fast.median_ms = 10.0;
        fast.p95_ms = 10.0;
        fast.wer = 0.02;
        assert_eq!(select_winner(&[fast, assessment()]), Some(1));
    }
}
