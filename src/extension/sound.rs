//! 확장 소리(ADR-0040): 플레이 창의 확장이 `sound.play{ … }`로 음표를 넘기면 앱이 악기 소리로 합성해 재생한다.
//!
//! - 음은 MIDI 번호(60 = C4)나 음 이름(`"C4"`, `"F#3"`, `"Bb2"`)이다. `note`(하나), `notes`(차례로, 쉼표는 `"-"`),
//!   `chord`(함께) 중 하나와 `length`(초)·`instrument`·`volume`·`loop`를 받는다.
//! - 악기는 기본 파형(`sine`·`square`·`triangle`·`saw`)과 몇 가지 흉내(`piano`·`organ`·`pluck`·`bass`), 타악기
//!   (`kick`·`snare`·`hihat`, 음 높이는 무시)다. 파일은 읽지 않는다.
//! - 소리는 플레이 창(창마다 묶음 하나)에서만 난다. 창을 닫으면 그 묶음이 멈춘다. 블록 그림은 소리를 내지 않는다.
//! - 합성은 이 모듈이 하고(같은 음표면 같은 소리), 출력만 rodio가 한다. 출력 장치가 없으면 조용히 버린다.
//!
//! 오류 글은 모델이 읽고 고치므로 영어로 쓴다.

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

use mlua::{Lua, Table, Value};

/// 합성 샘플 레이트(모노).
pub const SAMPLE_RATE: u32 = 44_100;

/// 한 번에 재생하는 소리의 길이 상한(초).
pub const MAX_SECONDS: f32 = 10.0;

/// 한 번에 넘기는 음표 수 상한.
pub const MAX_NOTES: usize = 256;

/// 창 하나에서 함께 나는 소리 수 상한. 넘으면 가장 오래된 것을 멈춘다.
pub const MAX_PLAYING: usize = 16;

/// 악기.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instrument {
    Sine,
    Square,
    Triangle,
    Saw,
    Piano,
    Organ,
    Pluck,
    Bass,
    Kick,
    Snare,
    Hihat,
}

impl Instrument {
    pub const ALL: [(&'static str, Instrument); 11] = [
        ("sine", Self::Sine),
        ("square", Self::Square),
        ("triangle", Self::Triangle),
        ("saw", Self::Saw),
        ("piano", Self::Piano),
        ("organ", Self::Organ),
        ("pluck", Self::Pluck),
        ("bass", Self::Bass),
        ("kick", Self::Kick),
        ("snare", Self::Snare),
        ("hihat", Self::Hihat),
    ];

    fn named(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, instrument)| *instrument)
    }
}

/// 음표 하나: 시작(초), 길이(초), MIDI 음 높이(쉼표면 `None`).
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub at: f32,
    pub length: f32,
    pub pitch: Option<f32>,
}

/// 한 번 재생할 것.
#[derive(Clone, Debug, PartialEq)]
pub struct Score {
    pub instrument: Instrument,
    /// 0~1.
    pub volume: f32,
    pub notes: Vec<Note>,
    /// 끝나면 처음부터 다시(멈출 때까지).
    pub looped: bool,
}

impl Score {
    /// 전체 길이(초).
    pub fn duration(&self) -> f32 {
        self.notes
            .iter()
            .map(|note| note.at + note.length)
            .fold(0., f32::max)
    }
}

/// 음 이름(`C4`, `F#3`, `Bb2`, `c-1`)의 MIDI 번호.
pub fn parse_note(name: &str) -> Option<f32> {
    let mut chars = name.trim().chars().peekable();
    let base = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut shift = 0;
    while let Some(&c) = chars.peek() {
        match c {
            '#' | '♯' => shift += 1,
            'b' | '♭' => shift -= 1,
            _ => break,
        }
        chars.next();
    }
    let octave: i32 = chars.collect::<String>().parse().ok()?;
    let midi = (octave + 1) * 12 + base + shift;
    (0..=127).contains(&midi).then_some(midi as f32)
}

/// MIDI 음 높이의 주파수(Hz). 69 = A4 = 440Hz.
pub fn frequency(pitch: f32) -> f32 {
    440. * 2f32.powf((pitch - 69.) / 12.)
}

/// 악보를 소리(모노 샘플, -1~1)로 만든다.
pub fn synth(score: &Score) -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    // 타악기와 감쇠하는 악기는 음표 길이 뒤로도 조금 울린다.
    let tail = match score.instrument {
        Instrument::Kick | Instrument::Snare | Instrument::Hihat => 0.0,
        Instrument::Piano | Instrument::Pluck => 0.3,
        _ => 0.03,
    };
    let total = ((score.duration() + tail).min(MAX_SECONDS) * rate).ceil() as usize;
    let mut out = vec![0.0f32; total];
    for (ix, note) in score.notes.iter().enumerate() {
        let Some(pitch) = note.pitch else {
            continue;
        };
        let start = (note.at * rate) as usize;
        let voice = voice(score.instrument, frequency(pitch), note.length, ix as u32);
        for (offset, sample) in voice.into_iter().enumerate() {
            if let Some(slot) = out.get_mut(start + offset) {
                *slot += sample;
            }
        }
    }
    // 화음이 겹쳐도 깨지지 않게 부드럽게 누른다.
    let gain = score.volume.clamp(0., 1.);
    for sample in &mut out {
        *sample = (*sample * gain).tanh();
    }
    out
}

/// 음 하나. `seed`는 잡음·현을 음표마다 다르게 한다.
fn voice(instrument: Instrument, freq: f32, length: f32, seed: u32) -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    let duration = match instrument {
        Instrument::Kick => 0.35,
        Instrument::Snare => 0.2,
        Instrument::Hihat => 0.06,
        Instrument::Piano | Instrument::Pluck => length + 0.3,
        _ => length + 0.03,
    };
    let count = (duration.min(MAX_SECONDS) * rate) as usize;
    let mut noise = Noise(0x9e37_79b9 ^ seed.wrapping_mul(0x85eb_ca6b));
    if instrument == Instrument::Pluck {
        return pluck(freq, count, length, &mut noise);
    }
    let mut phase = 0.0f32;
    let mut previous = 0.0f32;
    (0..count)
        .map(|i| {
            let t = i as f32 / rate;
            let wave = |phase: f32| phase.fract();
            let sample = match instrument {
                Instrument::Sine => (TAU * wave(phase)).sin() * 0.5,
                Instrument::Square => (if wave(phase) < 0.5 { 1. } else { -1. }) * 0.25,
                Instrument::Triangle => (4. * (wave(phase) - 0.5).abs() - 1.) * 0.45,
                Instrument::Saw => (2. * wave(phase) - 1.) * 0.25,
                Instrument::Organ => {
                    ((TAU * phase).sin() + 0.5 * (TAU * 2. * phase).sin() + 0.3 * (TAU * 4. * phase).sin())
                        * 0.22
                }
                Instrument::Piano => {
                    ((TAU * phase).sin() + 0.4 * (TAU * 2. * phase).sin() + 0.15 * (TAU * 3. * phase).sin())
                        * 0.35
                        * (-t * 3.).exp()
                }
                Instrument::Bass => {
                    ((TAU * wave(phase)).sin() * 0.6 + (2. * wave(phase) - 1.) * 0.2) * (-t * 2.).exp()
                }
                Instrument::Kick => (TAU * phase).sin() * 0.8 * (-t * 12.).exp(),
                Instrument::Snare => {
                    (noise.next() * 0.5 + (TAU * phase).sin() * 0.3) * (-t * 22.).exp()
                }
                Instrument::Hihat => {
                    // 잡음의 차분: 높은 소리만 남긴다.
                    let raw = noise.next();
                    let high = raw - previous;
                    previous = raw;
                    high * 0.3 * (-t * 60.).exp()
                }
                Instrument::Pluck => 0.,
            };
            let step = match instrument {
                // 킥은 높이가 빠르게 내려간다.
                Instrument::Kick => (150. * (-t * 25.).exp() + 45.) / rate,
                Instrument::Snare => 180. / rate,
                _ => freq / rate,
            };
            phase += step;
            sample * envelope(instrument, t, length)
        })
        .collect()
}

/// 소리의 켜짐·꺼짐(딸깍 소리가 나지 않게).
fn envelope(instrument: Instrument, t: f32, length: f32) -> f32 {
    const ATTACK: f32 = 0.005;
    const RELEASE: f32 = 0.03;
    let rise = (t / ATTACK).min(1.);
    match instrument {
        Instrument::Kick | Instrument::Snare | Instrument::Hihat => rise,
        Instrument::Piano | Instrument::Pluck => {
            // 음표 길이가 끝나면 빨리 줄인다(건반을 뗀다).
            let after = (t - length).max(0.);
            rise * (-after * 12.).exp()
        }
        _ if t > length => rise * (1. - (t - length) / RELEASE).max(0.),
        _ => rise,
    }
}

/// 뜯는 현(Karplus–Strong).
fn pluck(freq: f32, count: usize, length: f32, noise: &mut Noise) -> Vec<f32> {
    let period = ((SAMPLE_RATE as f32 / freq.max(20.)) as usize).max(2);
    let mut ring: Vec<f32> = (0..period).map(|_| noise.next() * 0.5).collect();
    (0..count)
        .map(|i| {
            let ix = i % period;
            let next = (ix + 1) % period;
            let sample = ring[ix];
            ring[ix] = (ring[ix] + ring[next]) * 0.5 * 0.996;
            sample * envelope(Instrument::Pluck, i as f32 / SAMPLE_RATE as f32, length)
        })
        .collect()
}

/// 늘 같은 잡음(xorshift).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2. - 1.
    }
}

// --- Lua -------------------------------------------------------------------

/// `sound.play`의 표를 악보로 읽는다.
pub fn parse_score(spec: &Table) -> Result<Score, String> {
    let instrument = match spec.get::<Value>("instrument").map_err(|err| err.to_string())? {
        Value::Nil => Instrument::Square,
        Value::String(name) => {
            let name = name.to_string_lossy();
            Instrument::named(&name).ok_or_else(|| {
                let known: Vec<&str> = Instrument::ALL.iter().map(|(name, _)| *name).collect();
                format!("unknown instrument \"{name}\" (use {})", known.join(", "))
            })?
        }
        other => return Err(format!("instrument must be a name, got {}", other.type_name())),
    };
    let number = |key: &str, default: f32| -> Result<f32, String> {
        match spec.get::<Value>(key).map_err(|err| err.to_string())? {
            Value::Nil => Ok(default),
            Value::Integer(n) => Ok(n as f32),
            Value::Number(n) => Ok(n as f32),
            other => Err(format!("{key} must be a number, got {}", other.type_name())),
        }
    };
    let volume = number("volume", 0.5)?;
    if !(0.0..=1.0).contains(&volume) {
        return Err("volume must be between 0 and 1".into());
    }
    let length = number("length", 0.25)?;
    if !(length > 0. && length <= MAX_SECONDS) {
        return Err(format!("length must be between 0 and {MAX_SECONDS} seconds"));
    }
    let looped = matches!(spec.get::<Value>("loop"), Ok(Value::Boolean(true)));
    let mut notes = Vec::new();
    let note = spec.get::<Value>("note").map_err(|err| err.to_string())?;
    let sequence = spec.get::<Value>("notes").map_err(|err| err.to_string())?;
    let chord = spec.get::<Value>("chord").map_err(|err| err.to_string())?;
    let given = [&note, &sequence, &chord].iter().filter(|v| !v.is_nil()).count();
    if given != 1 {
        return Err("give exactly one of note, notes (one after another), or chord (together)".into());
    }
    if !note.is_nil() {
        notes.push(Note {
            at: 0.,
            length,
            pitch: pitch(&note)?,
        });
    } else if let Value::Table(list) = &sequence {
        let mut at = 0.;
        for item in list.sequence_values::<Value>() {
            let item = item.map_err(|err| err.to_string())?;
            // `{ "C4", 0.5 }`: 음과 그 길이.
            let (value, own) = match &item {
                Value::Table(pair) => (
                    pair.get::<Value>(1).map_err(|err| err.to_string())?,
                    match pair.get::<Value>(2).map_err(|err| err.to_string())? {
                        Value::Integer(n) => n as f32,
                        Value::Number(n) => n as f32,
                        Value::Nil => length,
                        other => {
                            return Err(format!("a note's length must be a number, got {}", other.type_name()));
                        }
                    },
                ),
                _ => (item.clone(), length),
            };
            if !(own > 0. && own <= MAX_SECONDS) {
                return Err(format!("a note's length must be between 0 and {MAX_SECONDS} seconds"));
            }
            notes.push(Note {
                at,
                length: own,
                pitch: pitch(&value)?,
            });
            at += own;
        }
    } else if let Value::Table(list) = &chord {
        for item in list.sequence_values::<Value>() {
            notes.push(Note {
                at: 0.,
                length,
                pitch: pitch(&item.map_err(|err| err.to_string())?)?,
            });
        }
    } else {
        return Err("notes and chord must be lists of notes".into());
    }
    if notes.is_empty() {
        return Err("no notes to play".into());
    }
    if notes.len() > MAX_NOTES {
        return Err(format!("too many notes (limit {MAX_NOTES})"));
    }
    let score = Score {
        instrument,
        volume,
        notes,
        looped,
    };
    if score.duration() > MAX_SECONDS {
        return Err(format!("the sound is longer than {MAX_SECONDS} seconds"));
    }
    Ok(score)
}

/// 음 하나: MIDI 번호, 음 이름, 또는 쉼표(`"-"`).
fn pitch(value: &Value) -> Result<Option<f32>, String> {
    match value {
        Value::Integer(n) if (0..=127).contains(n) => Ok(Some(*n as f32)),
        Value::Number(n) if (0.0..=127.0).contains(n) => Ok(Some(*n as f32)),
        Value::Integer(_) | Value::Number(_) => Err("MIDI note numbers are 0 to 127 (60 is C4)".into()),
        Value::String(name) => {
            let name = name.to_string_lossy();
            if name == "-" || name.eq_ignore_ascii_case("rest") {
                return Ok(None);
            }
            parse_note(&name)
                .map(Some)
                .ok_or_else(|| format!("\"{name}\" is not a note (use names like C4, F#3, Bb2, or MIDI numbers)"))
        }
        other => Err(format!("a note must be a name or a MIDI number, got {}", other.type_name())),
    }
}

/// 소리 묶음 자리: 플레이 창이면 그 묶음, 아니면(블록) 비어 있다. 읽은 뒤에 채운다(Luau는 전역을 읽을 때
/// 붙잡아 두므로 `sound` 표는 바꾸지 않고 이 자리만 바꾼다).
pub type Slot = std::rc::Rc<std::cell::Cell<Option<u64>>>;

/// Lua 상태에 `sound`를 둔다(파일을 읽기 전에). 자리가 비어 있으면 부를 때 알린다.
pub fn install(lua: &Lua) -> mlua::Result<Slot> {
    let slot: Slot = Slot::default();
    let sound = lua.create_table()?;
    let next = std::rc::Rc::new(std::cell::Cell::new(1u64));
    let playing = slot.clone();
    sound.raw_set(
        "play",
        lua.create_function(move |_, spec: Value| {
            let Some(group) = playing.get() else {
                return Err(mlua::Error::runtime(SILENT));
            };
            let Value::Table(spec) = spec else {
                return Err(mlua::Error::runtime(
                    "sound.play takes a table: sound.play{ note = \"C4\", length = 0.2 }",
                ));
            };
            let score = parse_score(&spec).map_err(|err| mlua::Error::runtime(format!("sound.play: {err}")))?;
            let id = next.get();
            next.set(id + 1);
            send(Command::Play {
                group,
                id,
                samples: synth(&score),
                looped: score.looped,
            });
            Ok(id)
        })?,
    )?;
    let stopping = slot.clone();
    sound.raw_set(
        "stop",
        lua.create_function(move |_, id: Option<u64>| {
            let Some(group) = stopping.get() else {
                return Err(mlua::Error::runtime(SILENT));
            };
            send(Command::Stop { group, id });
            Ok(())
        })?,
    )?;
    lua.globals().raw_set("sound", sound)?;
    Ok(slot)
}

const SILENT: &str = "sound works only in play (the game window); block drawings stay silent";

// --- 재생 ------------------------------------------------------------------

/// 소리 묶음(플레이 창 하나) 번호.
pub fn new_group() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 재생 스레드에 보내는 일.
#[derive(Debug)]
pub enum Command {
    Play {
        group: u64,
        id: u64,
        samples: Vec<f32>,
        looped: bool,
    },
    /// `id`가 없으면 묶음의 소리를 모두 멈춘다.
    Stop { group: u64, id: Option<u64> },
    Pause(u64),
    Resume(u64),
}

/// 재생 스레드에 보낸다. 처음 부를 때 스레드를 띄운다.
pub fn send(command: Command) {
    #[cfg(test)]
    {
        SENT.lock().unwrap().push(command);
    }
    #[cfg(not(test))]
    {
        static ENGINE: OnceLock<Mutex<Sender<Command>>> = OnceLock::new();
        let engine = ENGINE.get_or_init(|| Mutex::new(spawn_engine()));
        if let Ok(engine) = engine.lock() {
            engine.send(command).ok();
        }
    }
}

/// 시험에서 보낸 일.
#[cfg(test)]
pub static SENT: Mutex<Vec<Command>> = Mutex::new(Vec::new());

#[cfg_attr(test, allow(dead_code))]
fn spawn_engine() -> Sender<Command> {
    let (tx, rx) = channel::<Command>();
    std::thread::Builder::new()
        .name("extension-sound".into())
        .spawn(move || {
            // 출력 장치가 없으면 소리 없이 일만 받아 버린다.
            let device = match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(mut device) => {
                    device.log_on_drop(false);
                    Some(device)
                }
                Err(err) => {
                    eprintln!("extension sound: no output device: {err}");
                    None
                }
            };
            let mut playing: HashMap<u64, Vec<(u64, rodio::Player)>> = HashMap::new();
            while let Ok(command) = rx.recv() {
                let Some(device) = &device else {
                    continue;
                };
                for players in playing.values_mut() {
                    players.retain(|(_, player)| !player.empty());
                }
                match command {
                    Command::Play {
                        group,
                        id,
                        samples,
                        looped,
                    } => {
                        let players = playing.entry(group).or_default();
                        if players.len() >= MAX_PLAYING {
                            players.remove(0).1.stop();
                        }
                        let player = rodio::Player::connect_new(device.mixer());
                        let buffer = rodio::buffer::SamplesBuffer::new(
                            std::num::NonZero::new(1).unwrap(),
                            std::num::NonZero::new(SAMPLE_RATE).unwrap(),
                            samples,
                        );
                        if looped {
                            player.append(rodio::Source::repeat_infinite(buffer));
                        } else {
                            player.append(buffer);
                        }
                        players.push((id, player));
                    }
                    Command::Stop { group, id } => {
                        if let Some(players) = playing.get_mut(&group) {
                            players.retain(|(own, player)| {
                                let stop = id.is_none_or(|id| id == *own);
                                if stop {
                                    player.stop();
                                }
                                !stop
                            });
                        }
                    }
                    Command::Pause(group) => {
                        for (_, player) in playing.get(&group).into_iter().flatten() {
                            player.pause();
                        }
                    }
                    Command::Resume(group) => {
                        for (_, player) in playing.get(&group).into_iter().flatten() {
                            player.play();
                        }
                    }
                }
            }
        })
        .ok();
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names_are_midi_numbers() {
        assert_eq!(parse_note("C4"), Some(60.));
        assert_eq!(parse_note("A4"), Some(69.));
        assert_eq!(parse_note("F#3"), Some(54.));
        assert_eq!(parse_note("Bb2"), Some(46.));
        assert_eq!(parse_note("c-1"), Some(0.));
        assert_eq!(parse_note("H4"), None);
        assert_eq!(parse_note("C"), None);
        assert!((frequency(69.) - 440.).abs() < 0.01);
        assert!((frequency(81.) - 880.).abs() < 0.01);
    }

    #[test]
    fn every_instrument_sounds_and_stays_in_range() {
        for (name, instrument) in Instrument::ALL {
            let samples = synth(&Score {
                instrument,
                volume: 1.,
                notes: vec![Note {
                    at: 0.,
                    length: 0.2,
                    pitch: Some(60.),
                }],
                looped: false,
            });
            let peak = samples.iter().fold(0f32, |peak, s| peak.max(s.abs()));
            assert!(peak > 0.05, "{name} is silent");
            assert!(peak <= 1., "{name} clips");
            // 딸깍 소리가 나지 않게 0에서 시작한다.
            assert!(samples[0].abs() < 0.05, "{name} starts with a click");
        }
    }

    #[test]
    fn reads_scores_from_lua() {
        let lua = Lua::new();
        let score = |code: &str| -> Result<Score, String> {
            parse_score(&lua.load(code).eval::<Table>().map_err(|err| err.to_string())?)
        };
        let melody = score(r#"{ notes = { "C4", "-", { "G4", 0.5 } }, length = 0.25, instrument = "piano" }"#).unwrap();
        assert_eq!(melody.instrument, Instrument::Piano);
        assert_eq!(
            melody.notes,
            vec![
                Note { at: 0., length: 0.25, pitch: Some(60.) },
                Note { at: 0.25, length: 0.25, pitch: None },
                Note { at: 0.5, length: 0.5, pitch: Some(67.) },
            ]
        );
        assert_eq!(melody.duration(), 1.);
        let chord = score(r#"{ chord = { 60, 64, 67 } }"#).unwrap();
        assert!(chord.notes.iter().all(|note| note.at == 0.));
        assert!(score(r#"{ note = "C4", chord = { 60 } }"#).unwrap_err().contains("exactly one"));
        assert!(score(r#"{ note = "X9" }"#).unwrap_err().contains("not a note"));
        assert!(score(r#"{ note = 60, instrument = "tuba" }"#).unwrap_err().contains("unknown instrument"));
        assert!(score(r#"{ note = 60, length = 30 }"#).unwrap_err().contains("length"));
    }

    #[test]
    fn plays_only_in_a_group() {
        let lua = Lua::new();
        let slot = install(&lua).unwrap();
        let err = lua.load(r#"sound.play{ note = "C4" }"#).exec().unwrap_err().to_string();
        assert!(err.contains("only in play"), "{err}");
        let group = new_group();
        slot.set(Some(group));
        let id: u64 = lua.load(r#"return sound.play{ note = "C4", loop = true }"#).eval().unwrap();
        lua.load("sound.stop()").exec().unwrap();
        let sent = SENT.lock().unwrap();
        assert!(sent.iter().any(|command| matches!(
            command,
            Command::Play { group: g, id: i, looped: true, .. } if *g == group && *i == id
        )));
        assert!(sent.iter().any(|command| matches!(
            command,
            Command::Stop { group: g, id: None } if *g == group
        )));
    }
}
