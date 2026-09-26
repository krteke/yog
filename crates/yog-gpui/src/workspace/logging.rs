use std::{sync::Mutex, thread::ThreadId};

use log::{Level, LevelFilter, Log, Metadata, Record};
use tokio::sync::mpsc::UnboundedSender;

use super::{
    execution::WorkerMessage,
    messages::{MessageLevel, MessageSource, NewMessage},
};

static LOGGER: GuiLogger = GuiLogger {
    sinks: Mutex::new(Vec::new()),
};

struct GuiLogger {
    sinks: Mutex<Vec<(ThreadId, UnboundedSender<WorkerMessage>)>>,
}

pub fn init() {
    log::set_logger(&LOGGER).expect("GUI logger must be installed once");
    log::set_max_level(LevelFilter::Debug);
}

pub struct Capture(ThreadId);

pub fn capture(sender: UnboundedSender<WorkerMessage>) -> Capture {
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
            _ = sender.send(WorkerMessage::Log(message));
        }
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn routes_command_and_ffmpeg_records_to_separate_sources() {
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
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

        let WorkerMessage::Log(command) = receiver.try_recv().unwrap() else {
            panic!("expected a log")
        };
        assert_eq!(command.source, MessageSource::Command);
        assert_eq!(command.text, "executing command: ffmpeg -i input");

        let WorkerMessage::Log(ffmpeg) = receiver.try_recv().unwrap() else {
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
                let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
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
                let WorkerMessage::Log(message) = receiver.try_recv().unwrap() else {
                    panic!("expected a log")
                };
                assert_eq!(message.text, name);
                assert!(receiver.try_recv().is_err());
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
