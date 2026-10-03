use chrono::{Duration, NaiveDate};
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct UsageStats {
    pub total_words: u64,
    pub total_characters: u64,
    pub dictations: u64,
    pub minutes_spoken: f64,
    pub time_saved_minutes: f64,
    pub streak_days: u64,
    pub per_app: Vec<AppUsage>,
    pub per_day: Vec<DayUsage>,
}

#[derive(Debug, Serialize)]
pub struct AppUsage {
    pub process: String,
    pub application_name: String,
    pub words: u64,
    pub dictations: u64,
    pub minutes_spoken: f64,
}

#[derive(Debug, Serialize)]
pub struct DayUsage {
    pub date: String,
    pub words: u64,
    pub dictations: u64,
    pub minutes_spoken: f64,
}

pub(super) fn aggregate(conn: &Connection, today: NaiveDate) -> Result<UsageStats, String> {
    // Manual notes and imported recordings do not represent live dictation.
    let predicate = "source = 'dictation' AND duration_ms > 0";
    let (words, characters, dictations, ms): (u64, u64, u64, u64) = conn.query_row(
        &format!("SELECT COALESCE(SUM(word_count),0), COALESCE(SUM(character_count),0), COUNT(*), COALESCE(SUM(duration_ms),0) FROM history WHERE {predicate}"),
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).map_err(|e| e.to_string())?;
    let mut apps = conn.prepare(&format!("SELECT application_process, MAX(application_name), SUM(word_count), COUNT(*), SUM(duration_ms) FROM history WHERE {predicate} GROUP BY application_process ORDER BY SUM(word_count) DESC, application_process"))
        .map_err(|e| e.to_string())?;
    let per_app = apps
        .query_map([], |row| {
            Ok(AppUsage {
                process: row.get(0)?,
                application_name: row.get(1)?,
                words: row.get(2)?,
                dictations: row.get(3)?,
                minutes_spoken: row.get::<_, f64>(4)? / 60_000.0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut days = conn.prepare(&format!("SELECT date(created_at, 'localtime'), SUM(word_count), COUNT(*), SUM(duration_ms) FROM history WHERE {predicate} AND date(created_at, 'localtime') >= ?1 AND date(created_at, 'localtime') <= ?2 GROUP BY date(created_at, 'localtime') ORDER BY date(created_at, 'localtime')"))
        .map_err(|e| e.to_string())?;
    let start = today - Duration::days(29);
    let observed = days
        .query_map(params![start.to_string(), today.to_string()], |row| {
            Ok(DayUsage {
                date: row.get(0)?,
                words: row.get(1)?,
                dictations: row.get(2)?,
                minutes_spoken: row.get::<_, f64>(3)? / 60_000.0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let per_day = (0..30)
        .map(|offset| {
            let date = (start + Duration::days(offset)).to_string();
            if let Some(day) = observed.iter().find(|day| day.date == date) {
                DayUsage {
                    date,
                    words: day.words,
                    dictations: day.dictations,
                    minutes_spoken: day.minutes_spoken,
                }
            } else {
                DayUsage {
                    date,
                    words: 0,
                    dictations: 0,
                    minutes_spoken: 0.0,
                }
            }
        })
        .collect();
    // Count an uninterrupted streak ending today, or yesterday before the
    // user's first dictation today. The SQL includes older days than the chart.
    let mut activity = conn.prepare(&format!("SELECT DISTINCT date(created_at, 'localtime') FROM history WHERE {predicate} AND date(created_at, 'localtime') <= ?1 ORDER BY 1 DESC"))
        .map_err(|e| e.to_string())?;
    let dates = activity
        .query_map([today.to_string()], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut expected = today;
    let mut streak_days = 0;
    for date in dates {
        let date = NaiveDate::parse_from_str(&date.map_err(|e| e.to_string())?, "%Y-%m-%d")
            .map_err(|e| e.to_string())?;
        if streak_days == 0 && date == today - Duration::days(1) {
            expected = date;
        }
        if date != expected {
            break;
        }
        streak_days += 1;
        expected -= Duration::days(1);
    }
    Ok(UsageStats {
        total_words: words,
        total_characters: characters,
        dictations,
        minutes_spoken: ms as f64 / 60_000.0,
        time_saved_minutes: words as f64 / 40.0,
        streak_days,
        per_app,
        per_day,
    })
}
