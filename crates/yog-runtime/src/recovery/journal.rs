use super::{
    MismatchPolicy, RecoveryAction, RecoveryDifference, RecoveryDifferenceKind, RecoveryMode,
    RecoveryOptions, RecoveryRequest, RecoveryScope,
};
use crate::{
    Command, Options,
    diagnostics::Diagnostics,
    event::{RecoveryInfo, RunEvent, RunPhase, SkipReason, TaskResult},
    jsonl::WriteLine,
    output::Output,
    path::Absolute,
    recursive::TaskPaths,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter},
    path::{Path, PathBuf},
    time::SystemTime,
};
use yog_core::{
    ffmpeg::{
        decoding::DecodingBackend,
        plan::{Container, VideoAction},
    },
    path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Settings {
    #[serde(with = "path::native")]
    output_root: PathBuf,
    container: Option<Container>,
    video: VideoAction,
    decoding: DecodingBackend,
}

impl Settings {
    fn new(command: &Command) -> Result<Self> {
        Ok(Self {
            output_root: command.request.output.try_absolutize()?,
            container: command.request.container,
            video: command.request.video.clone(),
            decoding: command.request.decoding.clone(),
        })
    }

    fn output_for(&self, relative: &Path) -> PathBuf {
        let mut output = self.output_root.join(relative);
        if let Some(container) = &self.container {
            output.set_extension(container.extension());
        }
        output
    }

    fn differences(&self, other: &Self, result: &SavedResult) -> Vec<RecoveryDifference> {
        if matches!(
            result,
            SavedResult::Skipped {
                reason: SkipReason::NoVideo
            }
        ) {
            return Vec::new();
        }
        let mut differences = Vec::new();
        if matches!(
            result,
            SavedResult::Skipped {
                reason: SkipReason::UnsupportedContainer
            }
        ) {
            if self.container != other.container {
                differences.push(RecoveryDifference {
                    kind: RecoveryDifferenceKind::Container,
                    recorded: self
                        .container
                        .map_or("Automatic", Container::extension)
                        .into(),
                    current: other
                        .container
                        .map_or("Automatic", Container::extension)
                        .into(),
                });
            }
            return differences;
        }
        if self.output_root != other.output_root {
            differences.push(RecoveryDifference {
                kind: RecoveryDifferenceKind::OutputRoot,
                recorded: self.output_root.display().to_string(),
                current: other.output_root.display().to_string(),
            });
        }
        if self.container != other.container {
            differences.push(RecoveryDifference {
                kind: RecoveryDifferenceKind::Container,
                recorded: self
                    .container
                    .map_or("Automatic", Container::extension)
                    .into(),
                current: other
                    .container
                    .map_or("Automatic", Container::extension)
                    .into(),
            });
        }
        if self.video != other.video {
            differences.push(RecoveryDifference {
                kind: RecoveryDifferenceKind::VideoEncoding,
                recorded: describe_video(&self.video),
                current: describe_video(&other.video),
            });
        }
        if matches!(self.video, VideoAction::Encode(_)) && self.decoding != other.decoding {
            differences.push(RecoveryDifference {
                kind: RecoveryDifferenceKind::Decoding,
                recorded: self.decoding.to_string(),
                current: other.decoding.to_string(),
            });
        }
        differences
    }
}

#[derive(Serialize, Deserialize)]
struct Header {
    #[serde(with = "path::native")]
    input_root: PathBuf,
    settings: Vec<Settings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fingerprint {
    bytes: u64,
    modified: SystemTime,
}

impl Fingerprint {
    fn read(path: &Path) -> std::io::Result<Self> {
        let metadata = path.metadata()?;
        Ok(Self {
            bytes: metadata.len(),
            modified: metadata.modified()?,
        })
    }

    fn difference(
        recorded: Option<&Self>,
        path: &Path,
        kind: RecoveryDifferenceKind,
    ) -> Option<RecoveryDifference> {
        let current = Self::read(path);
        if recorded.is_some() && current.as_ref().ok() == recorded {
            return None;
        }
        let describe =
            |stamp: &Self| format!("{} bytes; modified {:?}", stamp.bytes, stamp.modified);
        Some(RecoveryDifference {
            kind,
            recorded: recorded.map_or_else(|| "Not recorded".into(), describe),
            current: current
                .as_ref()
                .map_or_else(|error| format!("Unavailable: {error}"), describe),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SavedResult {
    Running,
    Success,
    Skipped { reason: SkipReason },
    Failed { phase: RunPhase, timed_out: bool },
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedOutput {
    #[serde(with = "path::native")]
    path: PathBuf,
    fingerprint: Option<Fingerprint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    #[serde(with = "path::native")]
    relative: PathBuf,
    setting: usize,
    input: Option<Fingerprint>,
    output: SavedOutput,
    result: SavedResult,
    part: bool,
}

pub struct RecoveredTask {
    pub result: TaskResult,
    pub info: RecoveryInfo,
}

pub struct Recovery {
    path: PathBuf,
    input_root: PathBuf,
    overwrite_output: bool,
    header: Header,
    current_setting: usize,
    entries: HashMap<PathBuf, Entry>,
    options: RecoveryOptions,
    recovered: HashMap<usize, RecoveredTask>,
    cleanup: HashMap<usize, PathBuf>,
    choices: HashMap<(RecoveryScope, Vec<RecoveryDifferenceKind>), RecoveryAction>,
    file: Option<BufWriter<File>>,
    error: Option<anyhow::Error>,
}

impl Recovery {
    pub fn open(
        command: &Command,
        options: &Options,
        diagnostics: &Diagnostics<'_>,
    ) -> Result<Self> {
        let input_root = command
            .request
            .input
            .canonicalize()
            .context("cannot resolve input directory")?;
        let current = Settings::new(command)?;
        let path = options
            .recovery
            .path
            .as_deref()
            .unwrap_or(&current.output_root.join(".yog-state.jsonl"))
            .try_absolutize()?;

        let saved = match read_state(&path, diagnostics) {
            Ok(saved) => saved,
            Err(error) if options.recovery.mode == RecoveryMode::Restart => {
                diagnostics.warning(&format!(
                    "Discarding unreadable state on restart: {error:#}"
                ));
                None
            }
            Err(error) => return Err(error),
        };

        let (mut header, mut entries) = saved.unwrap_or_else(|| {
            (
                Header {
                    input_root: input_root.clone(),
                    settings: vec![current.clone()],
                },
                HashMap::new(),
            )
        });
        if options.recovery.mode == RecoveryMode::Restart {
            if header.input_root != input_root {
                entries.clear();
                header.settings.clear();
            }
            header.input_root = input_root.clone();
        }
        let current_setting = match header
            .settings
            .iter()
            .position(|settings| settings == &current)
        {
            Some(index) => index,
            None => {
                header.settings.push(current);
                header.settings.len() - 1
            }
        };
        Ok(Self {
            path,
            input_root,
            overwrite_output: command.request.overwrite_output,
            header,
            current_setting,
            entries,
            options: options.recovery.clone(),
            recovered: HashMap::new(),
            cleanup: HashMap::new(),
            choices: HashMap::new(),
            file: None,
            error: None,
        })
    }

    pub async fn classify(
        &mut self,
        tasks: &[TaskPaths],
        cancellation: &tokio_util::sync::CancellationToken,
        diagnostics: &Diagnostics<'_>,
    ) -> Result<()> {
        let current = self.header.settings[self.current_setting].clone();
        let mut restart_batch = self.options.mode == RecoveryMode::Restart;
        let mut root_forced = false;
        if self.header.input_root != self.input_root {
            let differences = vec![RecoveryDifference {
                kind: RecoveryDifferenceKind::InputRoot,
                recorded: self.header.input_root.display().to_string(),
                current: self.input_root.display().to_string(),
            }];
            match self
                .decide(
                    RecoveryScope::Batch,
                    self.input_root.clone(),
                    None,
                    differences,
                    cancellation,
                    diagnostics,
                )
                .await?
            {
                RecoveryAction::Abort => bail!("recovery stopped: input directory differs"),
                RecoveryAction::Skip => {
                    for index in 0..tasks.len() {
                        self.recovered.insert(
                            index,
                            RecoveredTask {
                                result: TaskResult::Skipped {
                                    reason: SkipReason::ResumeMismatch,
                                    message: "input directory differs".into(),
                                },
                                info: RecoveryInfo {
                                    forced: false,
                                    output: None,
                                },
                            },
                        );
                    }
                    return Ok(());
                }
                RecoveryAction::Force => root_forced = true,
                RecoveryAction::Restart => {
                    self.entries.clear();
                    self.header.settings = vec![current.clone()];
                    self.current_setting = 0;
                    restart_batch = true;
                }
            }
            self.header.input_root = self.input_root.clone();
        }
        for (index, task) in tasks.iter().enumerate() {
            tokio::task::yield_now().await;
            if cancellation.is_cancelled() {
                return Ok(());
            }
            let relative = &task.relative;

            let Some(entry) = self.entries.get(relative).cloned() else {
                continue;
            };

            if restart_batch {
                break;
            }

            let differences =
                self.header.settings[entry.setting].differences(&current, &entry.result);
            let mut forced = root_forced;
            let mut mismatch_skip = None;

            if !differences.is_empty() {
                let names = differences
                    .iter()
                    .map(|difference| difference.kind.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                match self
                    .decide(
                        RecoveryScope::Batch,
                        task.input.clone(),
                        Some(entry.output.path.clone()),
                        differences,
                        cancellation,
                        diagnostics,
                    )
                    .await?
                {
                    RecoveryAction::Abort => bail!(
                        "state settings differ for {}: {}",
                        task.input.display(),
                        names
                    ),
                    RecoveryAction::Restart => {
                        restart_batch = true;
                        break;
                    }
                    RecoveryAction::Skip => {
                        mismatch_skip = Some(format!("settings differ: {names}"))
                    }
                    RecoveryAction::Force => forced = true,
                }
            }
            let reusable = matches!(
                entry.result,
                SavedResult::Success
                    | SavedResult::Skipped {
                        reason: SkipReason::NoVideo | SkipReason::UnsupportedContainer
                    }
            );
            let mut redo = !reusable;
            if mismatch_skip.is_none() {
                let mut differences: Vec<_> = Fingerprint::difference(
                    entry.input.as_ref(),
                    &task.input,
                    RecoveryDifferenceKind::SourceFingerprint,
                )
                .into_iter()
                .collect();
                if matches!(entry.result, SavedResult::Success) {
                    differences.extend(Fingerprint::difference(
                        entry.output.fingerprint.as_ref(),
                        &entry.output.path,
                        RecoveryDifferenceKind::OutputFingerprint,
                    ));
                }
                if !differences.is_empty() {
                    match self
                        .decide(
                            RecoveryScope::File,
                            task.input.clone(),
                            Some(entry.output.path.clone()),
                            differences,
                            cancellation,
                            diagnostics,
                        )
                        .await?
                    {
                        RecoveryAction::Abort => bail!(
                            "file fingerprint differs for {}; choose a file mismatch policy",
                            task.input.display()
                        ),
                        RecoveryAction::Force => forced = true,
                        RecoveryAction::Skip => {
                            mismatch_skip = Some("source or output fingerprint differs".into())
                        }
                        RecoveryAction::Restart => redo = true,
                    }
                }
            }
            if let Some(message) = mismatch_skip {
                self.recovered.insert(
                    index,
                    RecoveredTask {
                        result: TaskResult::Skipped {
                            reason: SkipReason::ResumeMismatch,
                            message,
                        },
                        info: RecoveryInfo {
                            forced: false,
                            output: matches!(entry.result, SavedResult::Success)
                                .then(|| entry.output.path.clone()),
                        },
                    },
                );
            } else if !redo {
                let reason = match entry.result {
                    SavedResult::Success => SkipReason::AlreadyCompleted,
                    SavedResult::Skipped { reason } => reason,
                    _ => unreachable!(),
                };
                self.recovered.insert(
                    index,
                    RecoveredTask {
                        result: TaskResult::Skipped {
                            reason,
                            message: if forced {
                                "Trusted previous record despite recovery differences".into()
                            } else {
                                "Reused previous task record".into()
                            },
                        },
                        info: RecoveryInfo {
                            forced,
                            output: Some(entry.output.path.clone()),
                        },
                    },
                );
            } else if entry.part {
                self.cleanup
                    .insert(index, Output::part_path(&entry.output.path)?);
            }
        }
        if restart_batch {
            self.recovered.clear();
            self.cleanup.clear();
            for (index, task) in tasks.iter().enumerate() {
                if let Some(entry) = self.entries.get(&task.relative)
                    && entry.part
                {
                    self.cleanup
                        .insert(index, Output::part_path(&entry.output.path)?);
                }
            }
            self.entries.retain(|_, entry| entry.part);
        }
        Ok(())
    }

    async fn decide(
        &mut self,
        scope: RecoveryScope,
        input: PathBuf,
        output: Option<PathBuf>,
        differences: Vec<RecoveryDifference>,
        cancellation: &tokio_util::sync::CancellationToken,
        diagnostics: &Diagnostics<'_>,
    ) -> Result<RecoveryAction> {
        let policy = match scope {
            RecoveryScope::Batch => self.options.batch_mismatch,
            RecoveryScope::File => self.options.file_mismatch,
        };
        let action = match policy {
            MismatchPolicy::Abort => Some(RecoveryAction::Abort),
            MismatchPolicy::Force => Some(RecoveryAction::Force),
            MismatchPolicy::Skip => Some(RecoveryAction::Skip),
            MismatchPolicy::Restart => Some(RecoveryAction::Restart),
            MismatchPolicy::Ask => None,
        };
        if let Some(action) = action {
            return Ok(action);
        }
        let key = (
            scope,
            differences
                .iter()
                .map(|difference| difference.kind)
                .collect(),
        );
        if let Some(action) = self.choices.get(&key) {
            return Ok(*action);
        }
        let (reply, response) = tokio::sync::oneshot::channel();
        diagnostics.event(RunEvent::RecoveryDecisionRequired(RecoveryRequest {
            state_path: self.path.clone(),
            input,
            output,
            scope,
            differences,
            overwrite_output: self.overwrite_output,
            reply,
        }));
        let decision = tokio::select! {
            biased;
            _ = cancellation.cancelled() => bail!("recovery confirmation cancelled"),
            decision = response => decision.context("recovery confirmation is unavailable; choose an explicit unattended mismatch policy")?.map_err(anyhow::Error::msg)?,
        };
        if decision.apply_to_similar {
            self.choices.insert(key, decision.action);
        }
        Ok(decision.action)
    }

    pub fn recovered(&self) -> &HashMap<usize, RecoveredTask> {
        &self.recovered
    }

    pub fn start(&mut self, diagnostics: &Diagnostics<'_>) -> Result<()> {
        self.compact()?;
        let reused = self
            .recovered
            .values()
            .filter(|task| {
                matches!(
                    task.result,
                    TaskResult::Skipped {
                        reason: SkipReason::AlreadyCompleted,
                        ..
                    }
                )
            })
            .count();
        let forced = self
            .recovered
            .values()
            .filter(|task| task.info.forced)
            .count();
        diagnostics.recovery(&self.path, reused, forced);
        diagnostics.event(RunEvent::RecoveryPrepared {
            path: self.path.clone(),
            reused,
            forced,
        });
        Ok(())
    }

    pub fn take_recovered(&mut self) -> HashMap<usize, RecoveredTask> {
        std::mem::take(&mut self.recovered)
    }

    pub fn begin(&mut self, index: usize, task: &TaskPaths) -> Result<()> {
        if let Some(part) = self.cleanup.remove(&index) {
            match fs::remove_file(&part) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("cannot remove previous partial output {}", part.display())
                    });
                }
            }
        }
        let relative = &task.relative;
        self.entries.insert(
            relative.clone(),
            Entry {
                relative: relative.clone(),
                setting: self.current_setting,
                input: Fingerprint::read(&task.input).ok(),
                output: SavedOutput {
                    path: self.header.settings[self.current_setting].output_for(relative),
                    fingerprint: None,
                },
                result: SavedResult::Running,
                part: false,
            },
        );
        self.append(relative);
        Ok(())
    }

    pub fn part_started(&mut self, relative: &Path) -> bool {
        self.entries.get_mut(relative).expect("task has begun").part = true;
        self.append(relative);
        self.error.is_none()
    }

    pub fn published(&mut self, relative: &Path) {
        let entry = self.entries.get_mut(relative).expect("task has begun");
        entry.result = SavedResult::Success;
        entry.part = false;
        entry.output.fingerprint = Fingerprint::read(&entry.output.path).ok();
        self.append(relative);
    }

    pub fn finish(&mut self, relative: &Path, result: &TaskResult) {
        let entry = self.entries.get_mut(relative).expect("task has begun");
        if matches!(entry.result, SavedResult::Success) {
            return;
        }
        entry.result = match result {
            TaskResult::Success => SavedResult::Success,
            TaskResult::Skipped { reason, .. } => SavedResult::Skipped { reason: *reason },
            TaskResult::Failed {
                phase, timed_out, ..
            } => SavedResult::Failed {
                phase: *phase,
                timed_out: *timed_out,
            },
            TaskResult::Cancelled => SavedResult::Cancelled,
        };
        entry.part = entry.part
            && Output::part_path(&entry.output.path)
                .expect("output has filename")
                .exists();
        self.append(relative);
    }

    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    pub fn take_error(&mut self) -> Option<anyhow::Error> {
        self.error.take()
    }

    fn append(&mut self, relative: &Path) {
        if self.error.is_some() {
            return;
        }
        let file = self.file.as_mut().expect("state is prepared before tasks");
        let result = file.write_line(&self.entries[relative]);

        if let Err(error) = result {
            self.error = Some(
                anyhow::Error::from(error)
                    .context(format!("cannot save state {}", self.path.display())),
            );
        }
    }

    fn compact(&mut self) -> Result<()> {
        fs::create_dir_all(self.path.parent().expect("absolute state has parent"))?;
        let mut file = BufWriter::new(File::create(&self.path)?);
        file.write_line(&self.header)?;
        for entry in self.entries.values() {
            file.write_line(entry)?;
        }
        self.file = Some(file);
        Ok(())
    }
}

fn describe_video(video: &VideoAction) -> String {
    use yog_core::ffmpeg::encoding::{RateControl, VideoEncoding};
    let VideoAction::Encode(encoding) = video else {
        return "Copy".into();
    };
    let mut parts = vec![encoding.to_string()];
    parts.push(format!(
        "Preset {}",
        encoding
            .preset()
            .map_or_else(|| "automatic".into(), |preset| preset.into_owned())
    ));
    if let Some(rate) = encoding.rate() {
        parts.push(match rate {
            RateControl::Quality(value) => format!("{} {value}", encoding.quality_parameter()),
            RateControl::Bitrate(value) => format!("{value} bit/s"),
        });
    }
    if let Some(multipass) = encoding.multipass() {
        parts.push(format!("Multipass {multipass}"));
    }
    if let VideoEncoding::Vaapi { device, .. } = encoding {
        parts.push(format!("Device {}", device.display()));
    }
    parts.join(" / ")
}

fn read_state(
    path: &Path,
    diagnostics: &Diagnostics<'_>,
) -> Result<Option<(Header, HashMap<PathBuf, Entry>)>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("cannot open state file"),
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let header: Header = serde_json::from_str(&line).context("invalid state header")?;
    let mut entries = HashMap::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let entry: Entry = match serde_json::from_str(&line) {
            Ok(entry) => entry,
            Err(error) if error.is_eof() && !line.ends_with('\n') => {
                diagnostics.warning("Ignoring an incomplete final state record");
                break;
            }
            Err(error) => return Err(error).context("invalid state task record"),
        };

        entries.insert(entry.relative.clone(), entry);
    }
    Ok(Some((header, entries)))
}

#[cfg(test)]
mod tests;
