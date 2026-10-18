#![allow(clippy::expect_used)]

use restichub_proto::agent::v1::step_result::Output;

use super::{Accepted, Collector, OutputKind, read_lines};

const BACKUP_1: &str = include_str!("../../tests/fixtures/restic-0.18.1/backup-1.json");
const BACKUP_3: &str = include_str!("../../tests/fixtures/restic-0.18.1/backup-3.json");
const CHECK: &str = include_str!("../../tests/fixtures/restic-0.18.1/check.json");
const FORGET: &str = include_str!("../../tests/fixtures/restic-0.18.1/forget.json");
const SNAPSHOTS: &str = include_str!("../../tests/fixtures/restic-0.18.1/snapshots.json");
const STATS: &str = include_str!("../../tests/fixtures/restic-0.18.1/stats-restore-size.json");
const STATS_RAW: &str = include_str!("../../tests/fixtures/restic-0.18.1/stats-raw-data.json");

fn feed(kind: OutputKind, text: &str) -> (Vec<Accepted>, Option<Output>) {
    let mut collector = Collector::new(kind, "job-1", "step-1");
    let accepted = text
        .lines()
        .map(|line| collector.accept(line, false))
        .collect();
    (accepted, collector.finish())
}

#[test]
fn fr_ag_08_a_backup_gives_progress_and_a_typed_summary() {
    let (accepted, output) = feed(OutputKind::Backup, BACKUP_1);
    match &accepted[0] {
        Accepted::Progress(progress) => {
            assert_eq!(progress.job_id, "job-1");
            assert_eq!(progress.step_id, "step-1");
            assert_eq!(progress.total_files, 2);
            assert_eq!(progress.files_done, 2);
            assert_eq!(progress.total_bytes, 300_006);
            assert!((progress.percent_done - 1.0).abs() < f64::EPSILON);
        }
        other => panic!("the status line must give progress: {other:?}"),
    }
    assert_eq!(accepted[1], Accepted::Consumed);
    let Some(Output::BackupSummary(summary)) = output else {
        panic!("a backup must give a summary");
    };
    assert_eq!(summary.files_new, 2);
    assert_eq!(summary.files_changed, 0);
    assert_eq!(summary.files_unmodified, 0);
    assert_eq!(summary.data_added, 301_481);
    assert_eq!(summary.total_files_processed, 2);
    assert!(summary.total_duration_seconds > 0.0);
    assert_eq!(summary.snapshot_id.len(), 64);
    assert!(!summary.dry_run);
}

#[test]
fn fr_ag_08_a_backup_with_changed_files_counts_them() {
    let (_, output) = feed(OutputKind::Backup, BACKUP_3);
    let Some(Output::BackupSummary(summary)) = output else {
        panic!("a backup must give a summary");
    };
    assert_eq!(summary.files_changed, 1);
    assert_eq!(summary.files_unmodified, 1);
}

#[test]
fn fr_ag_08_a_backup_error_becomes_a_log_line_without_the_json() {
    let line = r#"{"message_type":"error","error":{"message":"permission denied"},"during":"archival","item":"/srv/data/secret"}"#;
    let mut collector = Collector::new(OutputKind::Backup, "job-1", "step-1");
    assert_eq!(
        collector.accept(line, false),
        Accepted::Log("error during archival of /srv/data/secret: permission denied".to_owned())
    );
}

#[test]
fn adr_0011_unknown_fields_and_message_types_are_ignored() {
    let mut collector = Collector::new(OutputKind::Backup, "job-1", "step-1");
    let summary =
        r#"{"message_type":"summary","files_new":7,"future_field":{"a":1},"snapshot_id":"abc"}"#;
    assert_eq!(collector.accept(summary, false), Accepted::Consumed);
    assert_eq!(
        collector.accept(r#"{"message_type":"verbose_status","action":"new"}"#, false),
        Accepted::Status(r#"{"message_type":"verbose_status","action":"new"}"#.to_owned())
    );
    let Some(Output::BackupSummary(summary)) = collector.finish() else {
        panic!("the summary must survive an unknown field");
    };
    assert_eq!(summary.files_new, 7);
    assert_eq!(summary.snapshot_id, "abc");
}

#[test]
fn fr_ag_08_a_line_that_is_not_json_is_a_log_line() {
    let mut collector = Collector::new(OutputKind::Backup, "job-1", "step-1");
    assert_eq!(
        collector.accept("open repository", false),
        Accepted::Log("open repository".to_owned())
    );
    // A truncated line is never parsed, even when its start looks like JSON.
    let cut = r#"{"message_type":"summary","files_new":1"#;
    assert_eq!(collector.accept(cut, true), Accepted::Log(cut.to_owned()));
    assert!(collector.finish().is_none());
}

#[test]
fn fr_ag_08_a_check_gives_the_error_count() {
    let (_, output) = feed(OutputKind::Check, CHECK);
    let Some(Output::CheckSummary(summary)) = output else {
        panic!("a check must give a summary");
    };
    assert_eq!(summary.num_errors, 0);
    assert!(!summary.repair_suggested);

    let broken = r#"{"message_type":"summary","num_errors":3,"broken_packs":["ab"],"suggest_repair_index":true,"suggest_prune":false}"#;
    let (_, output) = feed(OutputKind::Check, broken);
    let Some(Output::CheckSummary(summary)) = output else {
        panic!("a check must give a summary");
    };
    assert_eq!(summary.num_errors, 3);
    assert!(summary.repair_suggested);
}

#[test]
fn fr_ag_08_snapshots_become_a_typed_list() {
    let (accepted, output) = feed(OutputKind::Snapshots, SNAPSHOTS);
    assert!(accepted.iter().all(|line| *line == Accepted::Consumed));
    let Some(Output::SnapshotList(list)) = output else {
        panic!("snapshots must give a list");
    };
    assert_eq!(list.snapshots.len(), 3);
    let first = &list.snapshots[0];
    assert_eq!(first.id.len(), 64);
    assert_eq!(first.short_id, first.id[..8]);
    assert_eq!(first.hostname, "host1");
    assert_eq!(first.username, "backup");
    assert_eq!(first.paths, vec!["/srv/data".to_owned()]);
    assert_eq!(first.tags, vec!["daily".to_owned()]);
    assert_eq!(first.program_version, "restic 0.18.1");
    assert!(first.time.as_ref().is_some_and(|time| time.seconds > 0));
    let summary = first.summary.as_ref().expect("the snapshot summary");
    assert_eq!(summary.files_new, 2);
    assert_eq!(summary.snapshot_id, first.id);
}

#[test]
fn fr_ag_08_forget_reports_the_kept_and_the_removed_snapshots() {
    let (_, output) = feed(OutputKind::Forget, FORGET);
    let Some(Output::ForgetResult(result)) = output else {
        panic!("forget must give a result");
    };
    assert_eq!(result.groups.len(), 1);
    let group = &result.groups[0];
    assert_eq!(group.host, "host1");
    assert_eq!(group.paths, vec!["/srv/data".to_owned()]);
    assert_eq!(group.kept.len(), 1);
    assert_eq!(group.removed.len(), 2);
    assert_eq!(group.kept[0].reason, "last snapshot");
    assert!(group.kept[0].time.is_some());
}

#[test]
fn fr_ag_08_stats_give_the_sizes() {
    let (_, output) = feed(OutputKind::Stats, STATS);
    let Some(Output::StatsSummary(stats)) = output else {
        panic!("stats must give a summary");
    };
    assert_eq!(stats.snapshots_count, 3);
    assert!(stats.total_size > 0);
    assert!(stats.total_file_count > 0);

    let (_, output) = feed(OutputKind::Stats, STATS_RAW);
    let Some(Output::StatsSummary(stats)) = output else {
        panic!("stats must give a summary");
    };
    assert!(stats.total_uncompressed_size >= stats.total_size);
    assert!(stats.compression_ratio >= 1.0);
}

#[test]
fn fr_ag_08_a_document_that_is_not_json_gives_no_output() {
    let (_, output) = feed(OutputKind::Snapshots, "Fatal: something broke");
    assert!(output.is_none());
}

#[test]
fn fr_ag_08_a_plain_command_logs_every_line() {
    let mut collector = Collector::new(OutputKind::Plain, "job-1", "step-1");
    assert_eq!(
        collector.accept("successfully removed 1 locks", false),
        Accepted::Log("successfully removed 1 locks".to_owned())
    );
    assert!(collector.finish().is_none());
}

#[test]
fn fr_ag_08_the_reader_bounds_the_line_length() {
    let mut data = vec![b'a'; 200];
    data.push(b'\n');
    data.extend_from_slice(b"short\r\nlast");
    let mut lines = Vec::new();
    read_lines(data.as_slice(), 50, |bytes, truncated| {
        lines.push((bytes, truncated));
        true
    });
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].0.len(), 50);
    assert!(lines[0].1, "the long line is marked as truncated");
    assert_eq!(lines[1], (b"short".to_vec(), false));
    assert_eq!(lines[2], (b"last".to_vec(), false));
}

#[test]
fn fr_rs_02_a_link_target_comes_from_the_tree_object_of_the_directory() {
    use restichub_proto::agent::v1::{LsPage, Node, NodeType};

    let node = |name: &str, kind: NodeType| Node {
        name: name.to_owned(),
        r#type: kind as i32,
        path: format!("/data/{name}"),
        permissions: "Lrwxrwxrwx".to_owned(),
        ..Node::default()
    };
    let mut page = LsPage {
        nodes: vec![
            node("current", NodeType::Symlink),
            node("docs", NodeType::Dir),
            node("old", NodeType::Symlink),
        ],
        last: true,
    };
    assert!(super::has_symlink(&page));

    let tree = br#"{"nodes":[{"name":"current","type":"symlink","linktarget":"docs"},{"name":"docs","type":"dir"}]}"#;
    super::fill_link_targets(&mut page, "/data", tree);
    assert_eq!(page.nodes[0].link_target, "docs");
    assert_eq!(page.nodes[1].link_target, "", "a directory has no target");
    assert_eq!(
        page.nodes[2].link_target, "",
        "a link that the tree does not name keeps an empty target"
    );

    // A tree that the agent cannot read changes nothing.
    super::fill_link_targets(&mut page, "/data", br#"{"nodes":"#);
    assert_eq!(page.nodes[0].link_target, "docs");

    let plain = LsPage {
        nodes: vec![node("docs", NodeType::Dir)],
        last: true,
    };
    assert!(
        !super::has_symlink(&plain),
        "a page without a link reads no tree"
    );
}

#[test]
fn fr_rs_02_a_recursive_page_matches_a_link_target_by_the_full_path() {
    use restichub_proto::agent::v1::{LsPage, Node, NodeType};

    let link = |path: &str| Node {
        name: "current".to_owned(),
        r#type: NodeType::Symlink as i32,
        path: path.to_owned(),
        ..Node::default()
    };
    let mut page = LsPage {
        nodes: vec![link("/data/current"), link("/data/sub/current")],
        last: true,
    };

    let tree = br#"{"nodes":[{"name":"current","type":"symlink","linktarget":"docs"}]}"#;
    super::fill_link_targets(&mut page, "/data", tree);
    assert_eq!(page.nodes[0].link_target, "docs");
    assert_eq!(
        page.nodes[1].link_target, "",
        "a link of a subdirectory takes no target of the listed directory"
    );
}

#[test]
fn fr_ag_08_the_reader_stops_when_the_consumer_stops() {
    let data = b"a\nb\nc\n";
    let mut count = 0;
    read_lines(data.as_slice(), 10, |_, _| {
        count += 1;
        false
    });
    assert_eq!(count, 1);
}

/// A snapshot document with long path lists, so the list is larger than the size limit.
fn large_snapshots(count: usize) -> String {
    let path = format!("/srv/{}", "p".repeat(2_000));
    let paths = vec![format!("\"{path}\""); 20].join(",");
    let items: Vec<String> = (0..count)
        .map(|index| {
            format!(
                r#"{{"id":"{index:0>64}","short_id":"{index:0>8}","time":"2026-01-01T00:00:00Z","paths":[{paths}],"hostname":"h","username":"u","tree":"t"}}"#
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

#[test]
fn fr_ag_08_a_large_snapshot_list_stays_inside_the_step_result_limit() {
    use prost::Message as _;
    use restichub_proto::agent::v1::StepResult;

    // 150 snapshots of about 40 KiB each are 6 MiB, which is over the limit.
    let (_, output) = feed(OutputKind::Snapshots, &large_snapshots(150));
    let Some(Output::SnapshotList(list)) = &output else {
        panic!("snapshots must give a list");
    };
    assert!(list.truncated);
    assert!(!list.snapshots.is_empty());
    // Snapshot 149 is the newest one.
    assert!(
        list.snapshots
            .last()
            .is_some_and(|snapshot| snapshot.short_id.ends_with("149"))
    );
    let result = StepResult {
        job_id: "job-1".to_owned(),
        step_id: "step-1".to_owned(),
        output,
        ..StepResult::default()
    };
    assert!(
        result.encoded_len() <= restichub_proto::MAX_STEP_RESULT_BYTES,
        "{}",
        result.encoded_len()
    );
}

#[test]
fn fr_ag_08_a_large_forget_result_stays_inside_the_step_result_limit() {
    use prost::Message as _;
    use restichub_proto::agent::v1::StepResult;

    let removed: Vec<String> = (0..1_500)
        .map(|index| {
            format!(
                r#"{{"id":"{index:0>64}","time":"2026-01-01T00:00:00Z","paths":["/srv"],"hostname":"h"}}"#
            )
        })
        .collect();
    let reasons: Vec<String> = (0..1_500)
        .map(|index| {
            format!(
                r#"{{"snapshot":{{"id":"{index:0>64}"}},"matches":["last snapshot","hourly snapshot {}"]}}"#,
                "x".repeat(2_000)
            )
        })
        .collect();
    let document = format!(
        r#"[{{"tags":null,"host":"h","paths":["/srv"],"keep":[],"remove":[{}],"reasons":[{}]}}]"#,
        removed.join(","),
        reasons.join(",")
    );
    let (_, output) = feed(OutputKind::Forget, &document);
    let Some(Output::ForgetResult(forget)) = &output else {
        panic!("forget must give a result");
    };
    assert!(forget.truncated);
    let result = StepResult {
        output,
        ..StepResult::default()
    };
    assert!(
        result.encoded_len() <= restichub_proto::MAX_STEP_RESULT_BYTES,
        "{}",
        result.encoded_len()
    );
}

#[test]
fn fr_ag_08_a_small_result_is_not_marked_truncated() {
    let (_, output) = feed(OutputKind::Snapshots, SNAPSHOTS);
    let Some(Output::SnapshotList(list)) = output else {
        panic!("snapshots must give a list");
    };
    assert!(!list.truncated);
    let (_, output) = feed(OutputKind::Forget, FORGET);
    let Some(Output::ForgetResult(forget)) = output else {
        panic!("forget must give a result");
    };
    assert!(!forget.truncated);
}

#[test]
fn fr_ag_08_a_document_step_keeps_a_short_line_queue() {
    assert!(OutputKind::Snapshots.queue_depth() <= 2);
    assert!(OutputKind::Forget.queue_depth() <= 2);
    assert!(OutputKind::Stats.queue_depth() <= 2);
    assert!(OutputKind::Backup.queue_depth() > 2);
}

#[test]
fn fr_ag_08_restic_json_errors_on_the_standard_error_become_readable_lines() {
    use super::stderr_text;

    assert_eq!(
        stderr_text(
            r#"{"message_type":"exit_error","code":12,"message":"Fatal: wrong password or no key found"}"#,
            false
        ),
        "Fatal: wrong password or no key found (exit code 12)"
    );
    assert_eq!(
        stderr_text(
            r#"{"message_type":"error","error":{"message":"permission denied"},"during":"archival","item":"/srv/a"}"#,
            false
        ),
        "error during archival of /srv/a: permission denied"
    );
    // Other text and other JSON stay as they are.
    assert_eq!(stderr_text("plain text", false), "plain text");
    assert_eq!(
        stderr_text(r#"{"message_type":"other"}"#, false),
        r#"{"message_type":"other"}"#
    );
    // A cut line may hold half a message, so the agent does not parse it.
    let cut = r#"{"message_type":"exit_error","code":1,"mess"#;
    assert_eq!(stderr_text(cut, true), cut);
}
