// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! The sandbox plugin code runs in (PR 32): WebAssembly components run by
//! wasmtime, each call in an instance of its own with a memory limit and a
//! deadline, given exactly one file to read and nothing else. No WASI is
//! linked, so a plugin has no clock, randomness, environment, file system
//! or network, and one that imports any of them does not load.
//!
//! The first use is previews (`wit/preview.wit`): a plugin turns a file of
//! a format Oikome does not show (a camera's RAW file) into an image.

use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "preview",
    });
}

use bindings::oikome::preview::types::Origin as WitOrigin;

/// How often the engine's clock ticks; deadlines are counted in ticks.
const TICK: Duration = Duration::from_millis(50);
/// The most one `read` gives a plugin.
pub const MAX_READ: u32 = 8 * 1024 * 1024;

/// What a call may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The plugin's memory, in bytes.
    pub memory: usize,
    /// How long the call may run.
    pub time: Duration,
    /// The largest image it may give back, in bytes.
    pub output: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory: 256 * 1024 * 1024,
            time: Duration::from_secs(5),
            output: 64 * 1024 * 1024,
        }
    }
}

/// Why a plugin gave no result.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Not a component of the world asked for, or one that imports
    /// something the sandbox does not give.
    #[error("the plugin's code can't be loaded: {0}")]
    Load(String),
    #[error("the plugin ran longer than {0:?}")]
    Time(Duration),
    #[error("the plugin needed more than {0} MiB of memory")]
    Memory(usize),
    #[error("the plugin gave back more than {0} MiB")]
    Output(usize),
    /// It trapped: a bug in the plugin.
    #[error("the plugin failed: {0}")]
    Trap(String),
    /// It ran and said it could not do this file.
    #[error("{0}")]
    Refused(String),
    /// What it gave back is not what was asked for.
    #[error("the plugin's result is not usable: {0}")]
    Result(String),
}

/// The one file a call may read.
pub trait Source: Send {
    fn size(&self) -> u64;
    /// Up to `len` bytes from `offset`; fewer at the end.
    fn read(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, String>;
}

/// A file opened by the host: the plugin never sees its path.
pub struct FileSource {
    file: std::fs::File,
    size: u64,
}

impl FileSource {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let size = file.metadata()?.len();
        Ok(FileSource { file, size })
    }
}

impl Source for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        if offset >= self.size {
            return Ok(Vec::new());
        }
        let len = u64::from(len).min(self.size - offset) as usize;
        let mut out = vec![0; len];
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read_exact(&mut out))
            .map_err(|e| e.to_string())?;
        Ok(out)
    }
}

/// Bytes in memory as a file, for tests and files already read.
pub struct BytesSource(pub Vec<u8>);

impl Source for BytesSource {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    fn read(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        let start = (offset as usize).min(self.0.len());
        let end = start.saturating_add(len as usize).min(self.0.len());
        Ok(self.0[start..end].to_vec())
    }
}

/// How a preview was made from its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Embedded,
    Developed,
}

/// The image a preview plugin made of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// `image/jpeg`, `image/png` or `image/webp`, as its bytes say.
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
    pub origin: Origin,
    /// The EXIF orientation (1 to 8) to show it in, when given.
    pub orientation: Option<u8>,
}

/// What one call holds: its file, and what limits it.
struct Call {
    source: Box<dyn Source>,
    limits: StoreLimits,
}

impl bindings::oikome::preview::source::Host for Call {
    fn size(&mut self) -> u64 {
        self.source.size()
    }

    fn read(&mut self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        self.source.read(offset, len.min(MAX_READ))
    }
}

impl bindings::oikome::preview::types::Host for Call {}

/// The engine every plugin runs on, and its clock.
pub struct Sandbox {
    engine: Engine,
}

impl Sandbox {
    /// The sandbox of this process: one engine, whose clock a thread ticks.
    pub fn shared() -> Result<Arc<Sandbox>, SandboxError> {
        static SHARED: OnceLock<Result<Arc<Sandbox>, String>> = OnceLock::new();
        SHARED
            .get_or_init(|| Sandbox::new().map(Arc::new).map_err(|e| e.to_string()))
            .clone()
            .map_err(SandboxError::Load)
    }

    fn new() -> Result<Sandbox, SandboxError> {
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(|e| SandboxError::Load(e.to_string()))?;
        // The clock ticks as long as the process runs, as the engine does.
        let clock = engine.clone();
        std::thread::Builder::new()
            .name("oikome-sandbox-clock".into())
            .spawn(move || loop {
                std::thread::sleep(TICK);
                clock.increment_epoch();
            })
            .map_err(|e| SandboxError::Load(e.to_string()))?;
        Ok(Sandbox { engine })
    }

    /// Compile a preview plugin's component. It is checked here: code that
    /// is not a component, or imports what the sandbox does not give, is
    /// refused before any file is given to it.
    pub fn preview_plugin(&self, wasm: &[u8]) -> Result<PreviewPlugin, SandboxError> {
        let component =
            Component::new(&self.engine, wasm).map_err(|e| SandboxError::Load(format!("{e:#}")))?;
        let mut linker: Linker<Call> = Linker::new(&self.engine);
        bindings::Preview::add_to_linker::<Call, HasSelf<Call>>(&mut linker, |call| call)
            .map_err(|e| SandboxError::Load(format!("{e:#}")))?;
        // Every import must be one the linker has: nothing else is given.
        linker
            .instantiate_pre(&component)
            .map_err(|e| SandboxError::Load(format!("{e:#}")))?;
        Ok(PreviewPlugin {
            engine: self.engine.clone(),
            component,
            linker: Arc::new(linker),
        })
    }
}

/// A preview plugin's compiled code, ready to be called.
#[derive(Clone)]
pub struct PreviewPlugin {
    engine: Engine,
    component: Component,
    linker: Arc<Linker<Call>>,
}

impl PreviewPlugin {
    /// An image of `source`, a file of `format`, made in an instance of its
    /// own within `limits`. Blocks while the plugin runs.
    pub fn render(
        &self,
        source: Box<dyn Source>,
        format: &str,
        limits: Limits,
    ) -> Result<Preview, SandboxError> {
        let call = Call {
            source,
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory)
                .instances(8)
                .memories(8)
                .tables(8)
                .trap_on_grow_failure(true)
                .build(),
        };
        let mut store = Store::new(&self.engine, call);
        store.limiter(|call| &mut call.limits);
        let ticks = (limits.time.as_millis() / TICK.as_millis()).max(1) as u64;
        store.set_epoch_deadline(ticks + 1);
        store.epoch_deadline_trap();
        let failed = |e: wasmtime::Error| {
            let text = format!("{e:#}");
            if matches!(
                e.downcast_ref::<wasmtime::Trap>(),
                Some(wasmtime::Trap::Interrupt)
            ) {
                SandboxError::Time(limits.time)
            } else if text.contains("forcing trap when growing memory")
                || text.contains("memory minimum size")
                || matches!(
                    e.downcast_ref::<wasmtime::Trap>(),
                    Some(wasmtime::Trap::AllocationTooLarge)
                )
            {
                SandboxError::Memory(limits.memory / 1024 / 1024)
            } else {
                SandboxError::Trap(text)
            }
        };
        let instance = bindings::Preview::instantiate(&mut store, &self.component, &self.linker)
            .map_err(failed)?;
        let image = instance
            .call_render(&mut store, format)
            .map_err(failed)?
            .map_err(SandboxError::Refused)?;
        if image.bytes.len() > limits.output {
            return Err(SandboxError::Output(limits.output / 1024 / 1024));
        }
        // What the bytes are is read from them, not taken on the plugin's word.
        let content_type = sniff(&image.bytes).ok_or_else(|| {
            SandboxError::Result("it is not a JPEG, PNG or WebP image".to_string())
        })?;
        if content_type != image.content_type {
            return Err(SandboxError::Result(format!(
                "it is {content_type}, not the {} it says",
                image.content_type
            )));
        }
        Ok(Preview {
            content_type,
            bytes: image.bytes,
            origin: match image.origin {
                WitOrigin::Embedded => Origin::Embedded,
                WitOrigin::Developed => Origin::Developed,
            },
            orientation: image.orientation.filter(|o| (1..=8).contains(o)),
        })
    }
}

/// The image type `bytes` start as.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}
