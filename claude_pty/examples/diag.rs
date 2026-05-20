use claude_pty::{ClaudeCode, Event};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = ClaudeCode::builder()
        .cwd(std::env::current_dir()?)
        .open()?;
    let mut stream = session.take_events().unwrap();
    let mut prompt_sent = false;
    let start = std::time::Instant::now();
    let mut last_event = std::time::Instant::now();

    loop {
        let elapsed = start.elapsed().as_secs_f32();
        if elapsed > 30.0 {
            eprintln!("Timeout — giving up after 30s");
            return Err("timeout after 30s".into());
        }

        let next = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            stream.next(),
        )
        .await;

        let evt = match next {
            Ok(Some(evt)) => evt,
            Ok(None) => break,
            Err(_) => continue,
        };

        let gap = last_event.elapsed().as_secs_f32();
        last_event = std::time::Instant::now();
        match &evt {
            Event::TuiToolConfirmation { message } => {
                eprintln!("[{elapsed:.1}s +{gap:.1}s] CONFIRM: {message}");
                session.write_raw(b"\r")?;
            }
            Event::TuiPrompt => {
                eprintln!("[{elapsed:.1}s +{gap:.1}s] >>> PROMPT DETECTED <<<");
                if !prompt_sent {
                    // eprintln!("[{elapsed:.1}s] Sending: say hi in one word");
                    // session.send_line("say hi in one word")?;
                    eprintln!("[{elapsed:.1}s] Sending: hello, what model are we using?");
                    session.send_line("hello, what model are we using?")?;
                    prompt_sent = true;
                } else {
                    eprintln!("[{elapsed:.1}s] Second prompt — SUCCESS, exiting.");
                    break;
                }
            }
            Event::TuiScreen { lines, cursor_row, cursor_col, .. } => {
                if gap > 0.5 {
                    eprintln!("[{elapsed:.1}s +{gap:.1}s] SCREEN (cursor={cursor_row},{cursor_col}):");
                    for line in lines {
                        eprintln!("  | {line}");
                    }
                }
            }
            Event::TuiOutput { text, cursor_row, cursor_col, .. } => {
                if gap > 0.5 || text.contains('❯') {
                    let preview: String = text.chars().take(120).collect();
                    eprintln!("[{elapsed:.1}s +{gap:.1}s] OUTPUT (cursor={cursor_row},{cursor_col}, has_❯={}): {preview:?}", text.contains('❯'));
                }
            }
            Event::LibDone => {
                eprintln!("[{elapsed:.1}s +{gap:.1}s] DONE");
                break;
            }
            Event::LibError { message } => {
                eprintln!("[{elapsed:.1}s +{gap:.1}s] ERROR: {message}");
                break;
            }
            _ => {}
        }
    }
    Ok(())
}
