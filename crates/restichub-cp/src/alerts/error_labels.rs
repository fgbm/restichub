//! The words for the stable error codes of a job, for the alerts (NFR-DOC-04).
//!
//! The user interface holds the same words as `jobs.errorCode.*` in its message catalogs. The test
//! in this module fails when the two lists differ.

/// The code, the English words and the Russian words.
pub(super) const LABELS: &[(&str, &str, &str)] = &[
    (
        "interrupted",
        "interrupted by an agent restart",
        "прервано перезапуском агента",
    ),
    ("step_failed", "restic error", "ошибка restic"),
    (
        "step_not_started",
        "restic did not start",
        "restic не запустился",
    ),
    (
        "wrong_password",
        "wrong password or keys",
        "неверный пароль или ключи",
    ),
    (
        "repository_missing",
        "repository not found",
        "репозиторий не найден",
    ),
    (
        "hook_failed",
        "script or hook error",
        "ошибка скрипта или хука",
    ),
    ("stalled", "no progress", "нет прогресса"),
    (
        "max_duration_exceeded",
        "maximum duration exceeded",
        "превышена предельная длительность",
    ),
    (
        "repository_locked",
        "repository locked",
        "репозиторий заблокирован",
    ),
    ("agent_lost", "the host went offline", "хост отключился"),
    ("expired", "the job expired", "срок задания истёк"),
    ("cancelled", "cancelled", "отменено"),
    (
        "request_missing",
        "the job lost its request",
        "у задания нет запроса",
    ),
    (
        "listing_missing",
        "the host sent no result",
        "хост не прислал результат",
    ),
    (
        "compile_failed",
        "the profile does not compile",
        "профиль не компилируется",
    ),
    (
        "lock_retries_exhausted",
        "the repository stayed locked",
        "репозиторий остался заблокирован",
    ),
    (
        "lock_retry",
        "locked, a new attempt is queued",
        "заблокирован, новая попытка в очереди",
    ),
    (
        "signed_job_unavailable",
        "the stored job is not readable",
        "сохранённое задание нечитаемо",
    ),
    (
        "succeeded_with_warnings",
        "finished with warnings",
        "завершено с предупреждениями",
    ),
    (
        "timed_out",
        "the job ran too long",
        "задание выполнялось слишком долго",
    ),
    (
        "unacknowledged_expired",
        "the host never confirmed the job",
        "хост не подтвердил задание",
    ),
    ("agent_revoked", "the host is revoked", "хост отозван"),
    (
        "not_acknowledged",
        "the host did not confirm the job",
        "хост не подтвердил задание",
    ),
    (
        "secret_unavailable",
        "a secret is not available",
        "секрет недоступен",
    ),
    (
        "signing_failed",
        "the job could not be signed",
        "не удалось подписать задание",
    ),
    ("plan_missing", "the plan is deleted", "план удалён"),
    (
        "plan_changed",
        "the plan moved to another host",
        "план перенесён на другой хост",
    ),
    (
        "profile_missing",
        "the profile is deleted",
        "профиль удалён",
    ),
    (
        "repository_deleted",
        "the repository is deleted",
        "репозиторий удалён",
    ),
    (
        "repository_changed",
        "the profile uses another repository",
        "профиль использует другой репозиторий",
    ),
    (
        "agent_facts_invalid",
        "the facts of the host are not usable",
        "сведения о хосте непригодны",
    ),
    (
        "forget_selected_nothing",
        "forget selected no snapshots",
        "forget не выбрал ни одного снимка",
    ),
    (
        "agent_capability_missing",
        "the agent is too old for the job",
        "агент слишком старый для задания",
    ),
    (
        "restore_target_denied",
        "the restore target is denied",
        "цель восстановления запрещена",
    ),
    (
        "repository_path_denied",
        "the repository path is not allowed on the host",
        "путь репозитория не разрешён на хосте",
    ),
    (
        "repository_read_only",
        "the repository path is read-only for the agent",
        "путь репозитория доступен агенту только для чтения",
    ),
    (
        "forget_without_tags",
        "forget without tags",
        "forget без тегов",
    ),
    (
        "step_missing",
        "the profile lacks the step",
        "в профиле нет шага",
    ),
    ("job_invalid", "the job is not valid", "задание некорректно"),
    (
        "unsupported_kind",
        "unsupported job kind",
        "неподдерживаемый вид задания",
    ),
    (
        "incompatible",
        "the agent restic does not support an option",
        "restic агента не поддерживает опцию",
    ),
];

#[cfg(test)]
mod tests {
    use super::LABELS;

    fn catalog(name: &str) -> String {
        let path = format!(
            "{}/../../ui/src/lib/i18n/{name}.ts",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).expect("cannot read the message catalog")
    }

    /// The `(code, text)` pairs of the keys with one prefix. A text may start on the next line.
    fn entries(catalog: &str, prefix: &str) -> Vec<(String, String)> {
        let key = format!("'{prefix}");
        let mut found = Vec::new();
        let mut lines = catalog.lines();
        while let Some(line) = lines.next() {
            let Some(rest) = line.trim().strip_prefix(&key) else {
                continue;
            };
            let (code, tail) = rest.split_once("':").expect("a catalog entry");
            let mut text = tail.trim().to_owned();
            if text.is_empty() {
                text = lines.next().expect("a catalog value").trim().to_owned();
            }
            let text = text.trim_end_matches(',').trim_matches('\'');
            found.push((code.to_owned(), text.to_owned()));
        }
        found
    }

    #[test]
    fn ac_mn_02_the_alert_labels_follow_the_error_codes_of_the_interface() {
        for name in ["en", "ru"] {
            let catalog = catalog(name);
            let mut interface = entries(&catalog, "jobs.errorCode.");
            interface.sort();
            let mut alerts: Vec<(String, String)> = LABELS
                .iter()
                .map(|row| {
                    let words = if name == "en" { row.1 } else { row.2 };
                    (row.0.to_owned(), words.to_owned())
                })
                .collect();
            alerts.sort();
            assert_eq!(alerts, interface, "the {name} labels differ");

            let reasons = entries(&catalog, "jobs.reason.");
            for (code, _) in reasons {
                assert!(
                    LABELS.iter().any(|row| row.0 == code),
                    "the code {code} has no alert label"
                );
            }
        }
    }
}
