//! The text of the alerts in the language of the installation (FR-MN-03, ADR-0035).
//!
//! An alert stores typed values, not a sentence. The email and webhook messages build the text
//! here, with the language of the setting `notifications.language`. The interface builds the same
//! sentences from the same values in the language of the user.

use restichub_db::JobKind;
use restichub_db::alerts::{Alert, AlertKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    En,
    Ru,
}

impl Language {
    /// `None` on a value that is not `en` or `ru`.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Self::En),
            "ru" => Some(Self::Ru),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ru => "ru",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Labels {
    pub tenant: &'static str,
    pub host: &'static str,
    pub plan: &'static str,
    pub repository: &'static str,
    pub details: &'static str,
    pub error: &'static str,
    pub opened: &'static str,
    pub resolved: &'static str,
    pub link: &'static str,
    pub resolved_headline: &'static str,
    pub test_subject: &'static str,
    pub test_body: &'static str,
}

pub fn labels(language: Language) -> Labels {
    match language {
        Language::En => Labels {
            tenant: "Tenant",
            host: "Host",
            plan: "Plan",
            repository: "Repository",
            details: "Details",
            error: "Error",
            opened: "Opened",
            resolved: "Resolved",
            link: "Link",
            resolved_headline: "Resolved",
            test_subject: "Test message",
            test_body: "This is a test message from ResticHub.",
        },
        Language::Ru => Labels {
            tenant: "Организация",
            host: "Хост",
            plan: "План",
            repository: "Репозиторий",
            details: "Подробности",
            error: "Ошибка",
            opened: "Открыт",
            resolved: "Закрыт",
            link: "Ссылка",
            resolved_headline: "Устранено",
            test_subject: "Тестовое сообщение",
            test_body: "Это тестовое сообщение от ResticHub.",
        },
    }
}

pub fn kind_title(language: Language, kind: AlertKind) -> &'static str {
    match language {
        Language::En => match kind {
            AlertKind::RpoViolated => "Backup is overdue",
            AlertKind::JobFailed => "Job failed",
            AlertKind::JobDeadLetter => "Job gave up after lock retries",
            AlertKind::JobWarnings => "Job finished with warnings",
            AlertKind::AgentOffline => "Host is offline",
            AlertKind::StaleLock => "Repository lock is stale",
        },
        Language::Ru => match kind {
            AlertKind::RpoViolated => "Резервная копия просрочена",
            AlertKind::JobFailed => "Задача завершилась ошибкой",
            AlertKind::JobDeadLetter => "Задача остановлена из-за блокировок",
            AlertKind::JobWarnings => "Задача завершилась с предупреждениями",
            AlertKind::AgentOffline => "Хост недоступен",
            AlertKind::StaleLock => "Блокировка репозитория устарела",
        },
    }
}

fn job_kind_name(language: Language, kind: Option<JobKind>) -> &'static str {
    let kind = kind.unwrap_or(JobKind::Other);
    match language {
        Language::En => match kind {
            JobKind::Init => "init",
            JobKind::TestAccess => "access test",
            JobKind::Backup => "backup",
            JobKind::Check => "check",
            JobKind::Forget => "delete expired snapshots",
            JobKind::Prune => "prune",
            JobKind::Restore => "restore",
            JobKind::Snapshots => "snapshot list",
            JobKind::Ls => "snapshot browsing",
            JobKind::Unlock => "unlock",
            JobKind::UnlockAll => "unlock all",
            JobKind::Other => "other",
        },
        Language::Ru => match kind {
            JobKind::Init => "инициализация",
            JobKind::TestAccess => "проверка доступа",
            JobKind::Backup => "резервное копирование",
            JobKind::Check => "проверка",
            JobKind::Forget => "удаление устаревших снимков",
            JobKind::Prune => "очистка репозитория",
            JobKind::Restore => "восстановление",
            JobKind::Snapshots => "список снимков",
            JobKind::Ls => "просмотр снимка",
            JobKind::Unlock => "снятие блокировки",
            JobKind::UnlockAll => "снятие всех блокировок",
            JobKind::Other => "прочее",
        },
    }
}

/// A short text for hours and minutes, such as `26 h` or `45 min`.
pub fn duration(language: Language, seconds: i64) -> String {
    let minutes = seconds / 60;
    let (hour, minute) = match language {
        Language::En => ("h", "min"),
        Language::Ru => ("ч", "мин"),
    };
    if minutes >= 120 {
        format!("{} {hour}", minutes / 60)
    } else {
        format!("{minutes} {minute}")
    }
}

/// The words for a stable error code of a job. `None` for a free text or an unknown code.
pub fn error_label(language: Language, code: &str) -> Option<&'static str> {
    let (_, en, ru) = super::error_labels::LABELS
        .iter()
        .find(|(known, _, _)| *known == code)?;
    Some(match language {
        Language::En => en,
        Language::Ru => ru,
    })
}

/// The sentence that tells why the alert exists. It holds no error text of a job: that text is a
/// separate field of the alert.
pub fn condition(language: Language, alert: &Alert) -> String {
    let params = &alert.params;
    let elapsed = duration(language, params.elapsed_seconds.unwrap_or(0));
    let rpo = duration(language, params.rpo_seconds.unwrap_or(0));
    let threshold = params.threshold_minutes.unwrap_or(0);
    let what = job_kind_name(language, params.job_kind);
    let code = params.error_code.as_deref().unwrap_or("failed");
    match (language, alert.kind) {
        (Language::En, AlertKind::RpoViolated) => match &params.last_success_at {
            Some(at) => format!(
                "No successful backup for {elapsed} (last success at {at}, objective {rpo})."
            ),
            None => format!("No successful backup since the plan was created (objective {rpo})."),
        },
        (Language::Ru, AlertKind::RpoViolated) => match &params.last_success_at {
            Some(at) => {
                format!("Нет успешной резервной копии уже {elapsed} (последняя: {at}, цель {rpo}).")
            }
            None => format!("Нет успешной копии с момента создания плана (цель {rpo})."),
        },
        (Language::En, AlertKind::AgentOffline) => {
            format!("The agent was not seen for {elapsed} (threshold {threshold} min).")
        }
        (Language::Ru, AlertKind::AgentOffline) => {
            format!("Агент не выходил на связь {elapsed} (порог {threshold} мин).")
        }
        (Language::En, AlertKind::StaleLock) => {
            format!("A repository lock has been stale for {elapsed} (threshold {threshold} min).")
        }
        (Language::Ru, AlertKind::StaleLock) => {
            format!("Блокировка репозитория устарела уже {elapsed} (порог {threshold} мин).")
        }
        (Language::En, AlertKind::JobDeadLetter) => {
            format!("The {what} job gave up after repeated repository lock retries.")
        }
        (Language::Ru, AlertKind::JobDeadLetter) => format!(
            "Задача «{what}» остановлена после повторных попыток из-за блокировки репозитория."
        ),
        (Language::En, AlertKind::JobWarnings) => {
            format!("The {what} job finished with warnings.")
        }
        (Language::Ru, AlertKind::JobWarnings) => {
            format!("Задача «{what}» завершилась с предупреждениями.")
        }
        (Language::En, AlertKind::JobFailed) if params.job_kind == Some(JobKind::Forget) => {
            format!("Deletion of expired snapshots failed ({code}).")
        }
        (Language::Ru, AlertKind::JobFailed) if params.job_kind == Some(JobKind::Forget) => {
            format!("Удаление устаревших снимков завершилось ошибкой ({code}).")
        }
        (Language::En, AlertKind::JobFailed) => format!("The {what} job failed ({code})."),
        (Language::Ru, AlertKind::JobFailed) => {
            format!("Задача «{what}» завершилась ошибкой ({code}).")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use restichub_db::AlertId;
    use restichub_db::alerts::{AlertParams, AlertStatus};
    use time::OffsetDateTime;

    #[test]
    fn the_language_parses_two_values() {
        assert_eq!(Language::parse("en"), Some(Language::En));
        assert_eq!(Language::parse(" RU "), Some(Language::Ru));
        assert_eq!(Language::parse("de"), None);
    }

    #[test]
    fn an_error_code_has_words_in_both_languages() {
        assert_eq!(
            error_label(Language::En, "interrupted"),
            Some("interrupted by an agent restart")
        );
        assert_eq!(
            error_label(Language::Ru, "interrupted"),
            Some("прервано перезапуском агента")
        );
        assert_eq!(error_label(Language::En, "restic said no"), None);
    }

    #[test]
    fn the_failed_forget_job_reads_as_a_sentence() {
        let alert = Alert {
            id: AlertId::new(),
            kind: AlertKind::JobFailed,
            dedupe_key: String::new(),
            status: AlertStatus::Open,
            agent_id: None,
            plan_id: None,
            repository_id: None,
            job_id: None,
            agent_hostname: None,
            plan_name: None,
            repository_name: None,
            params: sqlx::types::Json(AlertParams {
                job_kind: Some(JobKind::Forget),
                error_code: Some("timed_out".to_owned()),
                ..AlertParams::default()
            }),
            detail: String::new(),
            opened_at: OffsetDateTime::UNIX_EPOCH,
            resolved_at: None,
        };
        assert_eq!(
            condition(Language::En, &alert),
            "Deletion of expired snapshots failed (timed_out)."
        );
        assert_eq!(
            condition(Language::Ru, &alert),
            "Удаление устаревших снимков завершилось ошибкой (timed_out)."
        );
    }

    #[test]
    fn the_forget_job_has_the_ui_label() {
        assert_eq!(
            job_kind_name(Language::En, Some(JobKind::Forget)),
            "delete expired snapshots"
        );
        assert_eq!(
            job_kind_name(Language::Ru, Some(JobKind::Forget)),
            "удаление устаревших снимков"
        );
    }

    #[test]
    fn a_duration_uses_hours_from_two_hours() {
        assert_eq!(duration(Language::En, 45 * 60), "45 min");
        assert_eq!(duration(Language::En, 26 * 3600), "26 h");
        assert_eq!(duration(Language::Ru, 45 * 60), "45 мин");
        assert_eq!(duration(Language::Ru, 26 * 3600), "26 ч");
    }
}
