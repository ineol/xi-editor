// Copyright 2016 The xi-editor Authors.
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

//! A sample plugin, intended as an illustration and a template for plugin
//! developers.

#[macro_use]
extern crate log;

extern crate xi_core_lib as xi_core;
extern crate xi_plugin_lib;
extern crate xi_rope;
extern crate xi_rpc;

use std::fs::DirEntry;
use std::path::{Path, PathBuf};

use xi_core::plugin_rpc::{CompletionItem, CompletionResponse};
use xi_core::ConfigTable;
use xi_plugin_lib::{mainloop, ChunkCache, Error, Plugin, View};
use xi_rope::delta::Builder as EditBuilder;
use xi_rope::interval::Interval;
use xi_rope::rope::RopeDelta;
use xi_rpc::RemoteError;

/// A type that implements the `Plugin` trait, and interacts with xi-core.
///
/// Currently, this plugin has a single noteworthy behaviour,
/// intended to demonstrate how to edit a document; when the plugin is active,
/// and the user inserts an exclamation mark, the plugin will capitalize the
/// preceding word.
struct SamplePlugin;

//NOTE: implementing the `Plugin` trait is the sole requirement of a plugin.
// For more documentation, see `rust/plugin-lib` in this repo.
impl Plugin for SamplePlugin {
    type Cache = ChunkCache;

    fn new_view(&mut self, view: &mut View<Self::Cache>) {
        eprintln!("new view {}", view.get_id());
    }

    fn did_close(&mut self, view: &View<Self::Cache>) {
        eprintln!("close view {}", view.get_id());
    }

    fn did_save(&mut self, view: &mut View<Self::Cache>, _old: Option<&Path>) {
        eprintln!("saved view {}", view.get_id());
    }

    fn config_changed(&mut self, _view: &mut View<Self::Cache>, _changes: &ConfigTable) {}

    fn update(
        &mut self,
        view: &mut View<Self::Cache>,
        delta: Option<&RopeDelta>,
        _edit_type: String,
        _author: String,
    ) {
        //NOTE: example simple conditional edit. If this delta is
        //an insert of a single '!', we capitalize the preceding word.
        if let Some(delta) = delta {
            let (iv, _) = delta.summary();
            let text: String = delta.as_simple_insert().map(String::from).unwrap_or_default();
            if text == "!" {
                let _ = self.capitalize_word(view, iv.end());
            }
        }
    }

    /// Handles a request for autocomplete, by attempting to complete file paths.
    ///
    /// If the word under the cursor resembles a file path, this fn will attempt to
    /// locate that path and find subitems, which it will return as completion suggestions.
    fn completions(&mut self, view: &mut View<Self::Cache>, request_id: usize, pos: usize) {
        info!("completions called : pos={}", pos);
        let response = self.word_completions(view, pos).map(|items| CompletionResponse {
            is_incomplete: false,
            can_resolve: false,
            items,
        });

        view.completions(request_id, response)
    }
}

impl SamplePlugin {
    /// Uppercases the word preceding `end_offset`.
    fn capitalize_word(&self, view: &mut View<ChunkCache>, end_offset: usize) -> Result<(), Error> {
        //NOTE: this makes it clear to me that we need a better API for edits
        let line_nb = view.line_of_offset(end_offset)?;
        let line_start = view.offset_of_line(line_nb)?;

        let mut cur_utf8_ix = 0;
        let mut word_start = 0;
        for c in view.get_line(line_nb)?.chars() {
            if c.is_whitespace() {
                word_start = cur_utf8_ix;
            }

            cur_utf8_ix += c.len_utf8();

            if line_start + cur_utf8_ix == end_offset {
                break;
            }
        }

        let new_text = view.get_line(line_nb)?[word_start..end_offset - line_start].to_uppercase();
        let buf_size = view.get_buf_size();
        let mut builder = EditBuilder::new(buf_size);
        let iv = Interval::new(line_start + word_start, end_offset);
        builder.replace(iv, new_text.into());
        view.edit(builder.build(), 0, false, true, "sample".into());
        Ok(())
    }

    fn complete_word(word: &str, text: &str) -> Vec<String> {
        if word.len() == 0 {
            vec![]
        } else {
            let mut words = text
                .split(|c| !char::is_alphanumeric(c))
                .filter(|w| w.starts_with(&word) && w.len() > word.len())
                .map(|s| s.to_owned())
                .collect::<Vec<String>>();
            words.sort_unstable();
            words.dedup();
            words
        }
    }

    /// Attempts to find file path completion suggestions.
    fn word_completions(
        &self,
        view: &mut View<ChunkCache>,
        pos: usize,
    ) -> Result<Vec<CompletionItem>, RemoteError> {
        let (word_start, word) = self.get_word_at_offset(view, pos);
        let doc = match view.get_document() {
            Ok(doc) => doc,
            Err(e) => {
                info!("error: {:?}", e);
                "".to_owned()
            }
        };
        let completions = Self::complete_word(&word, &doc); // XXX
        Ok(self.make_completions(view, completions, &word, word_start))
    }

    /// Given a word to complete and a list of viable paths to suggest,
    /// constructs `CompletionItem`s.
    fn make_completions(
        &self,
        view: &View<ChunkCache>,
        words: Vec<String>,
        word: &str,
        word_off: usize,
    ) -> Vec<CompletionItem> {
        words
            .iter()
            .map(|w| {
                let mut completion = CompletionItem::with_label(w);
                let delta = RopeDelta::simple_edit(
                    Interval::new(
                        word_off, // XXX  start at begining of word or completion point?
                        word_off + word.len(),
                    ),
                    w.into(),
                    view.get_buf_size(),
                );
                completion.edit = Some(delta);
                completion
            })
            .collect()
    }

    fn get_word_at_offset(&self, view: &mut View<ChunkCache>, offset: usize) -> (usize, String) {
        let line_nb = view.line_of_offset(offset).unwrap();
        let line_start = view.offset_of_line(line_nb).unwrap();

        let mut cur_utf8_ix = 0;
        let mut word_start = 0;
        for c in view.get_line(line_nb).unwrap().chars() {
            if c.is_whitespace() {
                word_start = cur_utf8_ix;
            }

            if line_start + cur_utf8_ix == offset {
                break;
            }

            cur_utf8_ix += c.len_utf8();
        }

        let word = view
            .get_line(line_nb)
            .map(|s| s[word_start..offset - line_start].trim().to_string())
            .unwrap();
        eprintln!(
            "using word '{}' at line {} ({}..{})",
            &word,
            line_nb,
            word_start,
            offset - line_start
        );
        (word_start + line_start, word)
    }
}

use std::io;

fn create_log_directory(path_with_file: &Path) -> io::Result<()> {
    let log_dir = path_with_file.parent().ok_or_else(|| io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "Unable to get the parent of the following Path: {}, Your path should contain a file name",
            path_with_file.display(),
        ),
    ))?;
    std::fs::create_dir_all(log_dir)?;
    Ok(())
}

fn setup_logging(logging_path: Option<&Path>) -> Result<(), fern::InitError> {
    let level_filter = match std::env::var("XI_LOG") {
        Ok(level) => match level.to_lowercase().as_ref() {
            "trace" => log::LevelFilter::Trace,
            "debug" => log::LevelFilter::Debug,
            _ => log::LevelFilter::Info,
        },
        // Default to info
        Err(_) => log::LevelFilter::Info,
    };

    let mut fern_dispatch = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message,
            ))
        })
        .level(level_filter)
        .chain(io::stderr());

    if let Some(logging_file_path) = logging_path {
        create_log_directory(logging_file_path)?;

        fern_dispatch = fern_dispatch.chain(fern::log_file(logging_file_path)?);
    };

    // Start fern
    fern_dispatch.apply()?;
    info!("Logging with fern is set up");

    // Log details of the logging_file_path result using fern/log
    // Either logging the path fern is outputting to or the error from obtaining the path
    match logging_path {
        Some(logging_file_path) => info!("Writing logs to: {}", logging_file_path.display()),
        None => warn!("No path was supplied for the log file. Not saving logs to disk, falling back to just stderr"),
    }
    Ok(())
}

fn generate_logging_path() -> Result<PathBuf, io::Error> {
    // Use the file name set in logfile_config or fallback to the default
    let logfile_file_name = PathBuf::from("wordcomplete.log");

    let logfile_directory_name = PathBuf::from("xi-core");

    let mut logging_directory_path = get_logging_directory_path(logfile_directory_name)?;

    // Add the file name & return the full path
    logging_directory_path.push(logfile_file_name);
    Ok(logging_directory_path)
}

fn get_logging_directory_path<P: AsRef<Path>>(directory: P) -> Result<PathBuf, io::Error> {
    match dirs::data_local_dir() {
        Some(mut log_dir) => {
            log_dir.push(directory);
            Ok(log_dir)
        }
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "No standard logging directory known for this platform",
        )),
    }
}

fn main() {
    let logging_path_result = generate_logging_path();

    let logging_path =
        logging_path_result.as_ref().map(|p: &PathBuf| -> &Path { p.as_path() }).ok();
    setup_logging(logging_path);

    let mut plugin = SamplePlugin;
    mainloop(&mut plugin).unwrap();
}
