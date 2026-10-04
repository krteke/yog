use std::{sync::Mutex, thread::ThreadId};

use log::{Level, LevelFilter, Log, Metadata, Record};

use super::{
    execution::WorkerSender,
    messages::{MessageLevel, MessageSource, NewMessage},
};

static LOGGER: GuiLogger = GuiLogger {
    sinks: Mutex::new(Vec::new()),
};

struct GuiLogger {
    sinks: Mutex<Vec<(ThreadId, WorkerSender)>>,
}

pub fn init() {
    log::set_logger(&LOGGER).expect("GUI logger must be installed once");
    log::set_max_level(LevelFilter::Debug);
}

pub struct Capture(ThreadId);

pub(super) fn capture(sender: WorkerSender) -> Capture {
    let thread = std::thread::current().id();
    let mut sinks = LOGGER.sinks.lock().unwrap();
    assert!(
        sinks.iter().all(|(owner, _)| *owner != thread),
        "a worker thread must not capture logs twice"
    );
    sinks.push((thread, sender));
    Capture(thread)
}

impl Drop for Capture {
    fn drop(&mut self) {
        LOGGER
            .sinks
            .lock()
            .unwrap()
            .retain(|(thread, _)| *thread != self.0);
    }
}

impl Log for GuiLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Debug
            && (metadata.target().starts_with("yog_core")
                || metadata.target().starts_with("yog_runtime")
                || metadata.target().starts_with("yog::"))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let source = match record.target() {
            "yog::ffmpeg" => MessageSource::Ffmpeg,
            "yog::command" => MessageSource::Command,
            _ => MessageSource::Program,
        };
        let level = match record.level() {
            Level::Error => MessageLevel::Error,
            Level::Warn => MessageLevel::Warning,
            Level::Info => MessageLevel::Info,
            Level::Debug | Level::Trace => MessageLevel::Debug,
        };
        let message = NewMessage {
            source,
            level,
            text: record.args().to_string(),
        };
        if let Some((_, sender)) = self
            .sinks
            .lock()
            .unwrap()
            .iter()
            .find(|(thread, _)| *thread == std::thread::current().id())
        {
            sender.log(message);
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::super::execution::{self, WorkerMessage};
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn ffmpeg_log_flood_does_not_accumulate_an_unbounded_queue() {
        let (sender, mut receiver) = execution::channel();
        let capture = capture(sender);
        for index in 0..4_096 {
            LOGGER.log(
                &Record::builder()
                    .level(Level::Debug)
                    .target("yog::ffmpeg")
                    .args(format_args!("diagnostic {index}\n"))
                    .build(),
            );
        }
        drop(capture);
        let messages: Vec<_> = std::iter::from_fn(|| receiver.next_batch())
            .flatten()
            .collect();
        let WorkerMessage::Log(latest) = messages.last().unwrap() else {
            panic!("expected the latest diagnostic");
        };
        assert_eq!(latest.text, "diagnostic 4095\n");
        assert_eq!(messages.len(), 258);
        let WorkerMessage::Log(warning) = &messages[0] else {
            panic!("expected dropped-log warning");
        };
        assert_eq!(warning.level, MessageLevel::Warning);
        assert!(warning.text.contains("Dropped 3839 log chunks"));
    }

    #[test]
    fn routes_command_and_ffmpeg_records_to_separate_sources() {
        let (sender, mut receiver) = execution::channel();
        let _capture = capture(sender);

        LOGGER.log(
            &Record::builder()
                .level(Level::Debug)
                .target("yog::command")
                .args(format_args!("executing command: ffmpeg -i input"))
                .build(),
        );
        LOGGER.log(
            &Record::builder()
                .level(Level::Debug)
                .target("yog::ffmpeg")
                .args(format_args!("frame=10\n"))
                .build(),
        );

        let mut messages = receiver.next_batch().unwrap().into_iter();
        let WorkerMessage::Log(command) = messages.next().unwrap() else {
            panic!("expected a log")
        };
        assert_eq!(command.source, MessageSource::Command);
        assert_eq!(command.text, "executing command: ffmpeg -i input");

        let WorkerMessage::Log(ffmpeg) = messages.next().unwrap() else {
            panic!("expected a log")
        };
        assert_eq!(ffmpeg.source, MessageSource::Ffmpeg);
        assert_eq!(ffmpeg.text, "frame=10\n");
    }

    #[test]
    fn concurrent_workers_keep_their_logs_separate() {
        let barrier = Arc::new(Barrier::new(3));
        let worker = |name: &'static str, barrier: Arc<Barrier>| {
            std::thread::spawn(move || {
                let (sender, mut receiver) = execution::channel();
                let _capture = capture(sender);
                barrier.wait();
                LOGGER.log(
                    &Record::builder()
                        .level(Level::Info)
                        .target("yog_runtime")
                        .args(format_args!("{name}"))
                        .build(),
                );
                barrier.wait();
                let WorkerMessage::Log(message) = receiver.next_batch().unwrap().pop().unwrap()
                else {
                    panic!("expected a log")
                };
                assert_eq!(message.text, name);
                assert!(receiver.next_batch().is_none());
            })
        };
        let first = worker("predict", barrier.clone());
        let second = worker("emulate", barrier.clone());
        barrier.wait();
        barrier.wait();
        first.join().unwrap();
        second.join().unwrap();
    }
}
