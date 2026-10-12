use super::*;
use std::{
    ffi::OsString,
    io::Write,
    os::unix::ffi::OsStringExt,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use yog_core::ffmpeg::plan::TranscodeRequest;

struct Batch {
    root: PathBuf,
    command: Command,
    options: Options,
}

impl Batch {
    fn new() -> Self {
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "yog-state-test-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("input")).unwrap();
        fs::create_dir(root.join("output")).unwrap();
        fs::write(root.join("input/movie.mkv"), b"source").unwrap();
        Self {
            command: Command {
                request: TranscodeRequest::new(root.join("input"), root.join("output"))
                    .with_video(VideoAction::Copy),
                operation: crate::Operation::Transcode,
                recursive: true,
            },
            options: Options::default(),
            root,
        }
    }

    fn tasks(&self) -> Vec<TaskPaths> {
        vec![TaskPaths {
            relative: "movie.mkv".into(),
            input: self.command.request.input.join("movie.mkv"),
            output: self.command.request.output.join("movie.mkv"),
        }]
    }

    fn open(&self) -> Recovery {
        Recovery::open(
            &self.command,
            &self.options,
            &Diagnostics::new(false, false, &|_| {}),
        )
        .unwrap()
    }

    async fn unfinished(&self) {
        let mut state = self.open();
        let tasks = self.tasks();
        state
            .classify(&tasks, &tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();
        state
            .start(&Diagnostics::new(false, false, &|_| {}))
            .unwrap();
        state.begin(0, &tasks[0]).unwrap();
        let output = Output::prepare(&tasks[0].output, false).unwrap();
        assert!(state.part_started(&tasks[0].relative));
        fs::write(output.part(), b"unfinished").unwrap();
        // Simulate ordinary process termination leaving a journal-owned part.
        std::mem::forget(output);
    }

    async fn complete(&self) {
        let mut state = self.open();
        let tasks = self.tasks();
        state
            .classify(&tasks, &tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();
        state
            .start(&Diagnostics::new(false, false, &|_| {}))
            .unwrap();
        state.begin(0, &tasks[0]).unwrap();
        fs::write(&tasks[0].output, b"encoded").unwrap();
        state.published(&tasks[0].relative);
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[tokio::test]
async fn recovery_uses_each_completed_files_settings_instead_of_the_last_run() {
    let mut batch = Batch::new();
    batch.complete().await;
    batch.command.request.container = Some(yog_core::ffmpeg::plan::Container::Matroska);
    batch.options.recovery.batch_mismatch = MismatchPolicy::Force;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    drop(state);

    batch.command.request.container = None;
    batch.options.recovery.batch_mismatch = MismatchPolicy::Abort;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn changing_verification_does_not_invalidate_a_completed_video() {
    let mut batch = Batch::new();
    batch.complete().await;
    batch.options.verify = true;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn copy_recovery_ignores_decoder_changes() {
    let mut batch = Batch::new();
    batch.complete().await;
    batch.command.request.decoding = DecodingBackend::Vaapi(None);
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn non_video_recovery_ignores_encoding_changes() {
    let mut batch = Batch::new();
    let tasks = batch.tasks();
    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    state.finish(
        &tasks[0].relative,
        &TaskResult::Skipped {
            reason: SkipReason::NoVideo,
            message: "no video".into(),
        },
    );
    drop(state);

    batch.command.request.video = VideoAction::encode_x265(None, None);
    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn a_missing_output_directory_with_parent_components_can_be_recovered() {
    let mut batch = Batch::new();
    batch.command.request.output = batch.root.join("input/../new-output");
    batch.options.recovery.path = Some(batch.root.join("state.jsonl"));
    fs::create_dir_all(&batch.command.request.output).unwrap();
    // Resolve the state before the output directory exists, as the runtime does.
    fs::remove_dir(&batch.command.request.output).unwrap();
    let mut state = batch.open();
    let tasks = batch.tasks();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    fs::create_dir_all(&batch.command.request.output).unwrap();
    state.begin(0, &tasks[0]).unwrap();
    fs::write(&tasks[0].output, b"encoded").unwrap();
    state.published(&tasks[0].relative);
    drop(state);

    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[test]
fn output_path_normalization_does_not_require_traversable_components() {
    let mut batch = Batch::new();
    fs::write(batch.root.join("file"), b"not a directory").unwrap();
    batch.command.request.output = batch.root.join("file/../new-output");
    let settings = Settings::new(&batch.command).unwrap();
    assert_eq!(settings.output_root, batch.root.join("new-output"));
}

#[tokio::test]
async fn deleted_historical_inputs_do_not_trigger_batch_restart() {
    let mut batch = Batch::new();
    batch.complete().await;
    batch.command.request.container = Some(yog_core::ffmpeg::plan::Container::Matroska);
    batch.options.recovery.batch_mismatch = MismatchPolicy::Force;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    let task = TaskPaths {
        relative: "new.mkv".into(),
        input: batch.command.request.input.join("new.mkv"),
        output: batch.command.request.output.join("new.mkv"),
    };
    fs::write(&task.input, b"new source").unwrap();
    state.begin(1, &task).unwrap();
    fs::write(&task.output, b"encoded").unwrap();
    state.published(&task.relative);
    drop(state);
    fs::remove_file(batch.command.request.input.join("movie.mkv")).unwrap();

    batch.options.recovery.batch_mismatch = MismatchPolicy::Restart;
    let mut state = batch.open();
    state
        .classify(&[task], &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn a_complete_final_record_does_not_require_a_newline() {
    let batch = Batch::new();
    batch.complete().await;
    let path = batch.root.join("output/.yog-state.jsonl");
    let file = File::options().write(true).open(&path).unwrap();
    file.set_len(file.metadata().unwrap().len() - 1).unwrap();
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn state_stores_the_video_action_without_report_fields() {
    let batch = Batch::new();
    batch.complete().await;
    let state = fs::read_to_string(batch.root.join("output/.yog-state.jsonl")).unwrap();
    let header: serde_json::Value = serde_json::from_str(state.lines().next().unwrap()).unwrap();
    assert_eq!(
        header["settings"][0]["video"],
        serde_json::json!({"action": "copy"})
    );
}

#[tokio::test]
async fn force_does_not_accept_an_unknown_saved_encoder() {
    let mut batch = Batch::new();
    batch.complete().await;
    let path = batch.root.join("output/.yog-state.jsonl");
    let content = fs::read_to_string(&path).unwrap();
    let (header, entries) = content.split_once('\n').unwrap();
    let mut header: serde_json::Value = serde_json::from_str(header).unwrap();
    header["settings"][0]["video"] = serde_json::json!({"action":"encode", "encoder":"unknown"});
    fs::write(path, format!("{header}\n{entries}")).unwrap();
    batch.options.recovery.batch_mismatch = MismatchPolicy::Force;
    assert!(
        Recovery::open(
            &batch.command,
            &batch.options,
            &Diagnostics::new(false, false, &|_| {})
        )
        .is_err()
    );
}

#[tokio::test]
async fn state_rewriting_does_not_use_a_temporary_path() {
    let batch = Batch::new();
    let other = batch.root.join("output/.yog-state.jsonl.tmp");
    fs::create_dir(&other).unwrap();
    batch.complete().await;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
    assert!(other.is_dir());
}

#[tokio::test]
async fn a_state_write_error_after_publication_does_not_skip_requested_vmaf() {
    use crate::{
        report::record::{TaskRecord, TranscodeRecord},
        transcode::Transcoder,
    };
    use std::{
        os::{
            fd::OwnedFd,
            unix::{fs::PermissionsExt, net::UnixStream},
        },
        sync::Mutex,
    };

    let mut batch = Batch::new();
    let ffmpeg = batch.root.join("ffmpeg");
    fs::write(
        &ffmpeg,
        "#!/bin/sh\nfor last do :; done\nprintf encoded > \"$last\"\n",
    )
    .unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o700)).unwrap();
    batch.options.ffmpeg = ffmpeg;
    batch.options.vmaf = Some(yog_core::ffmpeg::vmaf::VmafOptions::default());
    let mut state = batch.open();
    let tasks = batch.tasks();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    let (writer, reader) = UnixStream::pair().unwrap();
    state.file = Some(BufWriter::new(File::from(OwnedFd::from(writer))));
    let reader = Mutex::new(Some(reader));
    let on_event = |event| {
        if matches!(event, RunEvent::PhaseChanged(RunPhase::Publishing)) {
            drop(reader.lock().unwrap().take());
        }
    };
    let diagnostics = Diagnostics::new(false, false, &on_event);
    let transcoder = Transcoder::new(
        &batch.options,
        crate::Config::default(),
        tokio_util::sync::CancellationToken::new(),
    );
    let request = yog_core::ffmpeg::plan::TranscodeRequest {
        input: tasks[0].input.clone(),
        output: tasks[0].output.clone(),
        ..batch.command.request.clone()
    };
    let media = serde_json::from_str(r#"{"streams":[{"index":0,"codec_type":"video"}],"format":{"format_name":"matroska,webm"}}"#).unwrap();
    let mut record = TranscodeRecord::new(&request);
    transcoder
        .run_probed(
            request,
            media,
            &diagnostics,
            &mut record,
            Some((&mut state, &tasks[0].relative)),
        )
        .await
        .unwrap();
    assert!(state.has_error());
    let record = serde_json::to_value(record.finish(TaskResult::Success)).unwrap();
    assert!(
        record["transcode"]["vmaf"]["error"].as_str().is_some(),
        "requested VMAF must run and report the missing reference dimensions: {record}"
    );
    assert_eq!(fs::read(&tasks[0].output).unwrap(), b"encoded");
}

#[tokio::test]
async fn explicit_restart_removes_only_the_recorded_unfinished_part() {
    let mut batch = Batch::new();
    batch.unfinished().await;
    fs::write(batch.root.join("output/.part.stranger.mkv"), b"unowned").unwrap();
    batch.options.recovery.mode = RecoveryMode::Restart;
    let mut state = batch.open();
    let tasks = batch.tasks();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    assert!(!Output::part_path(&tasks[0].output).unwrap().exists());
    assert!(batch.root.join("output/.part.stranger.mkv").exists());
}

#[tokio::test]
async fn batch_policy_restart_preserves_unfinished_part_ownership() {
    let mut batch = Batch::new();
    batch.unfinished().await;
    batch.command.request.container = Some(yog_core::ffmpeg::plan::Container::Matroska);
    batch.options.recovery.batch_mismatch = MismatchPolicy::Restart;
    let mut state = batch.open();
    let tasks = batch.tasks();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    assert!(!Output::part_path(&tasks[0].output).unwrap().exists());
}

#[tokio::test]
async fn an_unowned_existing_part_is_never_claimed_by_a_failed_attempt() {
    let batch = Batch::new();
    let tasks = batch.tasks();
    let part = Output::part_path(&tasks[0].output).unwrap();
    fs::write(&part, b"not ours").unwrap();
    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    assert!(Output::prepare(&tasks[0].output, false).is_err());
    state.finish(
        &tasks[0].relative,
        &TaskResult::Failed {
            phase: RunPhase::PreparingOutput,
            timed_out: false,
            message: "existing part".into(),
        },
    );
    drop(state);
    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    assert_eq!(fs::read(part).unwrap(), b"not ours");
}

#[tokio::test]
async fn partial_cleanup_failure_can_be_recorded_as_a_task_failure() {
    let mut batch = Batch::new();
    batch.unfinished().await;
    batch.options.recovery.mode = RecoveryMode::Restart;
    let tasks = batch.tasks();
    let part = Output::part_path(&tasks[0].output).unwrap();
    fs::remove_file(&part).unwrap();
    fs::create_dir(&part).unwrap();
    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    assert!(state.begin(0, &tasks[0]).is_err());
    state.finish(
        &tasks[0].relative,
        &TaskResult::Failed {
            phase: RunPhase::PreparingOutput,
            timed_out: false,
            message: "cannot clean partial output".into(),
        },
    );
    assert!(matches!(
        state.entries[Path::new("movie.mkv")].result,
        SavedResult::Failed {
            phase: RunPhase::PreparingOutput,
            ..
        }
    ));
    assert!(
        state.entries[Path::new("movie.mkv")].part,
        "cleanup failure must not relinquish the recorded partial output"
    );
}

#[tokio::test]
async fn incomplete_final_record_is_ignored_and_compacted_away() {
    let batch = Batch::new();
    batch.complete().await;
    let path = batch.root.join("output/.yog-state.jsonl");
    File::options()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"{\"relative\":")
        .unwrap();
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
    assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 2);
}

#[tokio::test]
async fn corrupt_complete_record_is_rejected() {
    let batch = Batch::new();
    batch.complete().await;
    File::options()
        .append(true)
        .open(batch.root.join("output/.yog-state.jsonl"))
        .unwrap()
        .write_all(b"{invalid}\n")
        .unwrap();
    let result = Recovery::open(
        &batch.command,
        &batch.options,
        &Diagnostics::new(false, false, &|_| {}),
    );
    assert!(
        result
            .err()
            .unwrap()
            .to_string()
            .contains("invalid state task record")
    );
}

#[tokio::test]
async fn fingerprints_compare_nanoseconds_even_when_size_is_unchanged() {
    let batch = Batch::new();
    let input = batch.root.join("input/movie.mkv");
    let file = File::options().write(true).open(&input).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::new(1_800_000_000, 1))
        .unwrap();
    batch.complete().await;
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::new(1_800_000_000, 2))
        .unwrap();
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    assert!(state.take_recovered().is_empty());
}

#[tokio::test]
async fn native_paths_round_trip_without_lossy_conversion() {
    let mut batch = Batch::new();
    let input = batch
        .root
        .join(std::ffi::OsString::from_vec(b"input-\xff".to_vec()));
    fs::rename(&batch.command.request.input, &input).unwrap();
    batch.command.request.input = input;
    batch.complete().await;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn vaapi_encoding_recovers_with_a_non_utf8_device_path() {
    use yog_core::ffmpeg::encoding::{RateControl, VideoCodec};
    let mut batch = Batch::new();
    batch.command.request.video = VideoAction::encode_vaapi(
        VideoCodec::Av1,
        OsString::from_vec(b"/dev/render-\xff".to_vec()),
        Some(RateControl::Quality(100)),
    );
    batch.complete().await;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn hardware_decoder_recovers_with_a_non_utf8_device_name() {
    let mut batch = Batch::new();
    batch.command.request.video = VideoAction::encode_x265(None, None);
    batch.command.request.decoding =
        DecodingBackend::Cuda(Some(OsString::from_vec(b"device-\xff".to_vec())));
    batch.complete().await;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn nvenc_recovery_retains_codec_bitrate_preset_and_multipass() {
    use std::num::NonZeroU64;
    use yog_core::ffmpeg::encoding::{NvencMultipass, NvencPreset, RateControl, VideoCodec};
    let mut batch = Batch::new();
    batch.command.request.video = VideoAction::encode_nvenc(
        VideoCodec::Hevc,
        Some(RateControl::Bitrate(NonZeroU64::new(4_000_000).unwrap())),
        Some(NvencPreset::P6),
        Some(NvencMultipass::QuarterResolution),
    );
    batch.complete().await;
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn aom_recovery_keeps_cpu_used_as_an_encoder_specific_integer() {
    let mut batch = Batch::new();
    batch.command.request.video = VideoAction::encode_aom_av1(None, Some(4));
    batch.complete().await;
    let content = fs::read_to_string(batch.root.join("output/.yog-state.jsonl")).unwrap();
    let header: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(
        header["settings"][0]["video"],
        serde_json::json!({"action":"encode", "encoder":"aom_av1", "rate":null, "cpu_used":4})
    );
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(state.take_recovered().len(), 1);
}

#[tokio::test]
async fn publication_remains_successful_when_the_state_write_fails() {
    let batch = Batch::new();
    let tasks = batch.tasks();
    let mut state = batch.open();
    state
        .classify(&tasks, &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    state.begin(0, &tasks[0]).unwrap();
    fs::write(&tasks[0].output, b"published").unwrap();
    state.file = Some(BufWriter::new(
        File::options().write(true).open("/dev/full").unwrap(),
    ));
    state.published(&tasks[0].relative);
    assert!(state.has_error());
    state.finish(&tasks[0].relative, &TaskResult::Cancelled);
    assert!(matches!(
        state.entries[Path::new("movie.mkv")].result,
        SavedResult::Success
    ));
    assert_eq!(fs::read(&tasks[0].output).unwrap(), b"published");
}

#[tokio::test]
async fn force_keeps_historical_settings_and_excludes_old_output_roots() {
    let mut batch = Batch::new();
    batch.complete().await;
    let old = batch.open().header.settings[0].clone();
    batch.options.recovery.path = Some(batch.root.join("output/.yog-state.jsonl"));
    batch.options.recovery.batch_mismatch = MismatchPolicy::Force;
    batch.command.request.output = batch.root.join("other-output");
    let mut state = batch.open();
    state
        .classify(&batch.tasks(), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
    let entry = &state.entries[Path::new("movie.mkv")];
    assert_eq!(state.header.settings[entry.setting], old);
    assert_ne!(entry.setting, state.current_setting);
    assert_eq!(
        state.take_recovered()[&0].info.output.as_deref(),
        Some(old.output_root.join("movie.mkv").as_path())
    );
    drop(state);
    fs::remove_file(batch.root.join("input/movie.mkv")).unwrap();
    let mut state = batch.open();
    state
        .classify(&[], &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    state
        .start(&Diagnostics::new(false, false, &|_| {}))
        .unwrap();
}
