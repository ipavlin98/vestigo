use super::docs::{Candidate, Outcome};
use anyhow::{Context, Result};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::io::{BufReader, ErrorKind, Read, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const WATCHDOG: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_millis(250);
const FRAME_HEADER: usize = 4 + 1 + 4;
const FLIGHTS_PER_THREAD: usize = 2;

pub(crate) type Stop = Box<dyn FnOnce() + Send>;
pub(crate) type Spawn = Arc<dyn Fn() -> Result<Pipe> + Send + Sync>;

pub(crate) struct Pipe {
    pub(crate) input: Box<dyn Write + Send>,
    pub(crate) output: Box<dyn Read + Send>,
    pub(crate) stop: Stop,
}

pub(super) fn request(id: u32, path: &str) -> String {
    format!("{id}\t{path}\n")
}

pub(super) fn parse_request(line: &str) -> Option<(u32, &str)> {
    let (id, path) = line.trim_end_matches(['\r', '\n']).split_once('\t')?;
    Some((id.parse().ok()?, path))
}

pub(super) fn write_frame(out: &mut impl Write, id: u32, outcome: Outcome, text: &[u8]) -> std::io::Result<()> {
    let mut header = Vec::with_capacity(FRAME_HEADER);
    header.extend_from_slice(&id.to_le_bytes());
    header.push(outcome.code());
    header.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.write_all(&header)?;
    out.write_all(text)?;
    out.flush()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Reply {
    pub(super) id: u32,
    pub(super) outcome: Outcome,
    pub(super) text: String,
}

pub(super) fn read_frame(input: &mut impl Read) -> std::io::Result<Option<Reply>> {
    let mut header = [0; FRAME_HEADER];
    let mut filled = 0;
    while filled < FRAME_HEADER {
        match input.read(header.get_mut(filled..).unwrap_or_default()) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    let [a, b, c, d, status, e, f, g, h] = header;
    let id = u32::from_le_bytes([a, b, c, d]);
    let length = u32::from_le_bytes([e, f, g, h]) as usize;
    let outcome = Outcome::of(status).ok_or(ErrorKind::InvalidData)?;
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    let text = String::from_utf8(bytes).map_err(|_| ErrorKind::InvalidData)?;
    Ok(Some(Reply { id, outcome, text }))
}

pub(super) struct Job {
    pub(super) candidate: Candidate,
    pub(super) replaces: Option<u32>,
}

pub(super) enum Step {
    Done(Job, Outcome, String),
    Idle,
}

enum Heard {
    Frame(Reply),
    Closed,
}

struct Process {
    input: Box<dyn Write + Send>,
    heard: Receiver<Heard>,
}

pub(super) struct Extractor<'a> {
    spawn: &'a Spawn,
    stopper: &'a Mutex<Option<Stop>>,
    process: Option<Process>,
    flights: usize,
}

impl<'a> Extractor<'a> {
    pub(super) fn new(spawn: &'a Spawn, stopper: &'a Mutex<Option<Stop>>) -> Self {
        let threads = std::thread::available_parallelism().map_or(1, |count| (count.get() / 2).max(1));
        Self { spawn, stopper, process: None, flights: FLIGHTS_PER_THREAD * threads }
    }

    fn start(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }
        let Pipe { input, output, stop } = (self.spawn)().context("starting the text extractor")?;
        *self.stopper.lock() = Some(stop);
        let (tell, heard) = mpsc::channel();
        let reading = std::thread::Builder::new().name("content-frames".into()).spawn(move || {
            let mut output = BufReader::new(output);
            while let Ok(Some(reply)) = read_frame(&mut output) {
                if tell.send(Heard::Frame(reply)).is_err() {
                    return;
                }
            }
            let _ = tell.send(Heard::Closed);
        });
        reading.context("reading the text extractor")?;
        self.process = Some(Process { input, heard });
        Ok(())
    }

    fn kill(&mut self) {
        self.process = None;
        if let Some(stop) = self.stopper.lock().take() {
            stop();
        }
    }

    pub(super) fn run(
        &mut self,
        jobs: Vec<Job>,
        alive: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(Step) -> Result<()>,
    ) -> Result<()> {
        let mut flight = Flight { queue: jobs.into(), ..Flight::default() };
        let mut quiet = Instant::now();
        while !flight.finished() {
            if !alive() {
                break;
            }
            self.start()?;
            let Some(process) = self.process.as_mut() else { continue };
            let sent = flight.send(self.flights, &mut process.input);
            let heard = if sent { process.heard.recv_timeout(TICK) } else { Ok(Heard::Closed) };
            let crashed = match heard {
                Ok(Heard::Frame(Reply { id, outcome, text })) => {
                    quiet = Instant::now();
                    if let Some(job) = flight.flying.remove(&id) {
                        sink(Step::Done(job, outcome, text))?;
                    }
                    false
                }
                Err(RecvTimeoutError::Timeout) => {
                    sink(Step::Idle)?;
                    quiet.elapsed() >= WATCHDOG
                }
                Ok(Heard::Closed) | Err(RecvTimeoutError::Disconnected) => true,
            };
            if crashed {
                log::warn!("The text extractor stopped answering, restarting it");
                self.kill();
                quiet = Instant::now();
                for job in flight.crashed() {
                    log::warn!("{}: the text extractor failed on it", job.candidate.path);
                    sink(Step::Done(job, Outcome::Failed, String::new()))?;
                }
            }
        }
        self.kill();
        Ok(())
    }
}

#[derive(Default)]
struct Flight {
    queue: VecDeque<Job>,
    suspects: VecDeque<Job>,
    flying: FxHashMap<u32, Job>,
    alone: bool,
    next: u32,
}

impl Flight {
    fn finished(&self) -> bool {
        self.queue.is_empty() && self.suspects.is_empty() && self.flying.is_empty()
    }

    fn send_one(&mut self, job: Job, input: &mut dyn Write) -> bool {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        let sent = input.write_all(request(id, &job.candidate.path).as_bytes()).and_then(|()| input.flush());
        self.flying.insert(id, job);
        sent.is_ok()
    }

    fn send(&mut self, flights: usize, input: &mut dyn Write) -> bool {
        self.alone = !self.suspects.is_empty() || self.alone && !self.flying.is_empty();
        if self.alone {
            if self.flying.is_empty()
                && let Some(job) = self.suspects.pop_front()
            {
                return self.send_one(job, input);
            }
            return true;
        }
        while self.flying.len() < flights {
            let Some(job) = self.queue.pop_front() else { break };
            if !self.send_one(job, input) {
                return false;
            }
        }
        true
    }

    fn crashed(&mut self) -> Vec<Job> {
        let flying = self.flying.drain().map(|(_, job)| job);
        if self.alone {
            self.alone = false;
            return flying.collect();
        }
        self.suspects.extend(flying);
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn job(path: &str) -> Job {
        Job { candidate: Candidate { path: path.to_owned(), size: 1, modified: 1 }, replaces: None }
    }

    #[test]
    fn requests_carry_any_path() {
        let line = request(7, "C:\\Папка с пробелом\\файл.txt");
        assert_eq!(parse_request(&line), Some((7, "C:\\Папка с пробелом\\файл.txt")));
        assert_eq!(parse_request("nonsense\n"), None);
        assert_eq!(parse_request("x\tC:\\a\n"), None);
    }

    #[test]
    fn frames_survive_the_pipe() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, 3, Outcome::Text, "текст".as_bytes()).expect("written");
        write_frame(&mut bytes, 4, Outcome::Encrypted, b"").expect("written");
        let mut input = Cursor::new(bytes);
        let reply = |id, outcome, text: &str| Reply { id, outcome, text: text.to_owned() };
        assert_eq!(read_frame(&mut input).expect("read"), Some(reply(3, Outcome::Text, "текст")));
        assert_eq!(read_frame(&mut input).expect("read"), Some(reply(4, Outcome::Encrypted, "")));
        assert_eq!(read_frame(&mut input).expect("read"), None);
    }

    #[test]
    fn a_cut_frame_is_an_error_not_a_clean_end() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, 3, Outcome::Text, b"text").expect("written");
        bytes.truncate(bytes.len() - 1);
        assert!(read_frame(&mut Cursor::new(&bytes)).is_err());
        bytes.truncate(2);
        assert!(read_frame(&mut Cursor::new(&bytes)).is_err());
    }

    #[test]
    fn a_crash_sends_the_flying_files_one_by_one_and_then_blames_the_last_one() {
        let mut flight = Flight { queue: [job("a"), job("b"), job("c")].into(), ..Flight::default() };
        let mut sink = Vec::new();
        assert!(flight.send(2, &mut sink));
        assert_eq!(flight.flying.len(), 2);
        assert!(flight.crashed().is_empty());
        assert_eq!(flight.suspects.len(), 2);
        assert!(flight.send(2, &mut sink));
        assert_eq!(flight.flying.len(), 1, "a suspect flies alone");
        let culprit = flight.crashed();
        assert_eq!(culprit.len(), 1);
        assert!(flight.send(2, &mut sink));
        let id = *flight.flying.keys().next().expect("the other suspect");
        flight.flying.remove(&id);
        assert!(flight.send(2, &mut sink));
        assert!(!flight.alone);
        assert_eq!(flight.queue.len(), 0);
        assert_eq!(flight.flying.len(), 1, "the queue goes on after the suspects");
    }
}
