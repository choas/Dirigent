use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::claude;

#[derive(Debug)]
pub(crate) enum CodexError { NotFound, SpawnFailed(std::io::Error), Cancelled, NonZeroExit(std::process::ExitStatus), InvalidExtraArgs(String) }
impl std::fmt::Display for CodexError { fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { match self { Self::NotFound=>write!(f,"codex CLI not found on PATH"), Self::SpawnFailed(e)=>write!(f,"failed to spawn codex: {e}"), Self::Cancelled=>write!(f,"cancelled"), Self::NonZeroExit(s)=>write!(f,"codex exited with {s}"), Self::InvalidExtraArgs(a)=>write!(f,"failed to parse extra_args: {a}") } } }
impl std::error::Error for CodexError {}
#[derive(Debug, Clone)]
pub(crate) struct CodexResponse { pub stdout: String, pub edited_files: Vec<String> }
pub(crate) struct CodexRunConfig<'a> { pub model: &'a str, pub cli_path: &'a str, pub extra_args: &'a str, pub env_vars: &'a str, pub pre_run_script: &'a str, pub post_run_script: &'a str }
fn run_hook_script(label:&str, script:&str, project_root:&Path, on_log:&mut impl FnMut(&str), fail_on_error:bool)->Result<(),CodexError>{ if script.trim().is_empty(){return Ok(());} on_log(&format!("▶ {}: {}\n",label,script.trim())); let result=Command::new("sh").arg("-c").arg(script.trim()).current_dir(project_root).output(); match result { Ok(output)=>{ if !output.stdout.is_empty(){on_log(&String::from_utf8_lossy(&output.stdout));} if !output.stderr.is_empty(){on_log(&String::from_utf8_lossy(&output.stderr));} if !output.status.success() && fail_on_error { return Err(CodexError::SpawnFailed(std::io::Error::other(format!("{} script failed ({})",label,output.status)))); } }, Err(e)=> if fail_on_error { return Err(CodexError::SpawnFailed(e)); } } Ok(()) }

pub(crate) fn invoke_codex_streaming(prompt:&str, project_root:&Path, config:&CodexRunConfig<'_>, mut on_log:impl FnMut(&str), cancel:Arc<AtomicBool>) -> Result<CodexResponse,CodexError>{
 if cancel.load(std::sync::atomic::Ordering::Relaxed){ return Err(CodexError::Cancelled); }
 let codex_bin=which::which(if config.cli_path.is_empty(){"codex"}else{config.cli_path}).map_err(|_|CodexError::NotFound)?;
 run_hook_script("Pre-run script", config.pre_run_script, project_root, &mut on_log, true)?;
 let mut cmd=Command::new(codex_bin);
 cmd.arg("--yolo");
 if !config.model.is_empty(){ cmd.arg("--model").arg(config.model); }
 if !config.extra_args.trim().is_empty(){ for a in shlex::split(config.extra_args).ok_or_else(||CodexError::InvalidExtraArgs(config.extra_args.to_string()))? { cmd.arg(a);} }
 cmd.arg(prompt);
 claude::apply_env_vars(&mut cmd, config.env_vars, &mut on_log);
 claude::apply_dirigent_env(&mut cmd, project_root, &mut on_log);
 let out=cmd.current_dir(project_root).output().map_err(CodexError::SpawnFailed)?;
 let stdout=String::from_utf8_lossy(&out.stdout).to_string(); let stderr=String::from_utf8_lossy(&out.stderr).to_string();
 if !stdout.is_empty(){on_log(&stdout);} if !stderr.is_empty(){on_log(&stderr);} if !out.status.success(){return Err(CodexError::NonZeroExit(out.status));}
 run_hook_script("Post-run script", config.post_run_script, project_root, &mut on_log, false)?;
 Ok(CodexResponse{ stdout: format!("{}{}",stdout, if stderr.is_empty(){""}else{"\n"})+&stderr, edited_files: Vec::new() }) }

pub(crate) fn parse_diff_from_response(_response:&str)->Option<String>{ None }
