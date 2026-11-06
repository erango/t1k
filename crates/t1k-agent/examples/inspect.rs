//! Print what T1K extracts from a transcript: `cargo run -p t1k-agent --example inspect -- <file.jsonl>`.

use t1k_agent::hooks::Hook;
use t1k_agent::AgentState;

fn main() {
    let path = std::env::args().nth(1).expect("usage: inspect <transcript.jsonl>");
    let mut a = AgentState::new("inspect".into());
    a.apply_hook(Hook::SessionStart { session_id: "inspect".into(), transcript_path: Some(path), cwd: None });
    while a.poll_transcript() {}
    println!("title:  {:?}", a.title());
    println!("branch: {:?}", a.branch);
    println!("recap:  {:?}", a.recap.as_deref().map(|r| r.chars().take(160).collect::<String>()));
    println!("outputs:");
    for o in &a.outputs {
        println!("  {:<9} {:<28} {}", o.kind.label(), o.label, o.target);
    }
}
