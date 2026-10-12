use super::{MismatchPolicy, RecoveryMode, RecoveryOptions};
use crate::{
    Command, Options,
    diagnostics::Diagnostics,
    event::{RecoveryInfo, RunEvent, RunPhase, SkipReason, TaskResult},
    jsonl::WriteLine,
    output::Output,
    path::Absolute,
    recursive::TaskPaths,
};
use anyhow::{Context, Result, bail, ensure};
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

    fn differences(&self, other: &Self, result: &SavedResult) -> Vec<&'static str> {
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
                differences.push("container");
            }
            return differences;
        }
        if self.output_root != other.output_root {
            differences.push("output root");
        }
        if self.container != other.container {
            differences.push("container");
        }
        if self.video != other.video {
            differences.push("video encoding");
        }
        if matches!(self.video, VideoAction::Encode(_)) && self.decoding != other.decoding {
            differences.push("decoding");
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
    header: Header,
    current_setting: usize,
    entries: HashMap<PathBuf, Entry>,
    options: RecoveryOptions,
    recovered: HashMap<usize, RecoveredTask>,
    cleanup: HashMap<usize, PathBuf>,
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
            header.input_root = input_root;
        } else {
            ensure!(
                header.input_root == input_root,
                "state input directory differs; choose Restart to start a new batch"
            );
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
            header,
            current_setting,
            entries,
            options: options.recovery.clone(),
            recovered: HashMap::new(),
            cleanup: HashMap::new(),
            file: None,
            error: None,
        })
    }

    pub async fn classify(
        &mut self,
        tasks: &[TaskPaths],
        cancellation: &tokio_util::sync::CancellationToken,
    ) -> Result<()> {
        let current = &self.header.settings[self.current_setting];
        let restart_batch = self.options.mode == RecoveryMode::Restart
            || self.options.batch_mismatch == MismatchPolicy::Restart
                && tasks
                    .iter()
                    .filter_map(|task| self.entries.get(&task.relative))
                    .any(|entry| {
                        !self.header.settings[entry.setting]
                            .differences(current, &entry.result)
                            .is_empty()
                    });
        for (index, task) in tasks.iter().enumerate() {
            tokio::task::yield_now().await;
            if cancellation.is_cancelled() {
                return Ok(());
            }
            let relative = &task.relative;

            let Some(entry) = self.entries.get(relative) else {
                continue;
            };

            if restart_batch {
                if entry.part {
                    self.cleanup
                        .insert(index, Output::part_path(&entry.output.path)?);
                }
                continue;
            }

            let differences =
                self.header.settings[entry.setting].differences(current, &entry.result);
            let mut forced = false;
            let mut mismatch_skip = None;

            if !differences.is_empty() {
                match self.options.batch_mismatch {
                    MismatchPolicy::Abort => bail!(
                        "state settings differ for {}: {}",
                        task.input.display(),
                        differences.join(", ")
                    ),
                    MismatchPolicy::Restart => {
                        unreachable!("batch restart is resolved before classification")
                    }
                    MismatchPolicy::Skip => {
                        mismatch_skip = Some(format!("settings differ: {}", differences.join(", ")))
                    }
                    MismatchPolicy::Force => forced = true,
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
                let input_matches = entry.input.as_ref().is_some_and(|stamp| {
                    Fingerprint::read(&task.input).ok().as_ref() == Some(stamp)
                });
                let output_matches = !matches!(entry.result, SavedResult::Success)
                    || entry.output.fingerprint.as_ref().is_some_and(|stamp| {
                        Fingerprint::read(&entry.output.path).ok().as_ref() == Some(stamp)
                    });
                if !input_matches || !output_matches {
                    match self.options.file_mismatch {
                        MismatchPolicy::Abort => bail!(
                            "file fingerprint differs for {}; choose a file mismatch policy",
                            task.input.display()
                        ),
                        MismatchPolicy::Force => forced = true,
                        MismatchPolicy::Skip => {
                            mismatch_skip = Some("source or output fingerprint differs".into())
                        }
                        MismatchPolicy::Restart => redo = true,
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
            self.entries.retain(|_, entry| entry.part);
        }
        Ok(())
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
