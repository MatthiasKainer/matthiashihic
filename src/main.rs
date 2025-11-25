//! matthiashihic - compiler for *.matthiashihic that uses OpenAI to execute pseudocode
//!
//! Usage:
//!   ./matthiashihic program.matthiashihic --api-key <OPENAI_API_KEY> [--model <MODEL_NAME>]
//!
//! Specification:
//!   hihi!                     -- required program header (first non-empty line)
//!   "text"                    -- only allowed statement; pseudocode to execute
//!   eat that java!            -- required terminator; stop parsing here
//!   anything after terminator -- ignored (comments)
//!
//! The compiler reads the pseudocode and sends it to OpenAI API for execution,
//! streaming the response back to stdout.

use std::env;
use std::fs;

fn usage_and_exit(program: &str) -> ! {
    let msg = format!(
        "Usage:
  {p} <source.matthiashihic> [--api-key <OPENAI_API_KEY>] [--model <MODEL_NAME>] [-o <output>]

Example:
  {p} hello.matthiashihic --api-key sk-... -o hello
  {p} hello.matthiashihic --model gpt-4o -o hello
  {p} hello.matthiashihic -o hello  # Uses OPENAI_API_KEY env var at runtime

Default model: gpt-4
API key priority: 1) OPENAI_API_KEY env var at runtime, 2) embedded key from --api-key
",
        p = program
    );
    eprint!("{}", msg);
    std::process::exit(2);
}

fn escape_rust_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn xor_encrypt(data: &str, key: &[u8]) -> Vec<u8> {
    data.as_bytes()
        .iter()
        .enumerate()
        .map(|(i, &b)| b ^ key[i % key.len()])
        .collect()
}

fn generate_xor_key() -> Vec<u8> {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("matthiashihic-{}", nanos).into_bytes()
}

fn process_placeholders(s: &str, required_args: &mut std::collections::HashSet<usize>) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    
    while let Some(ch) = chars.next() {
        if ch == '€' {
            if let Some(&next_ch) = chars.peek() {
                if next_ch == '€' {
                    // €€index -> €index (escape)
                    chars.next(); // consume the second €
                    result.push('€');
                } else if next_ch.is_ascii_digit() {
                    // €index -> placeholder
                    let mut num_str = String::new();
                    while let Some(&digit_ch) = chars.peek() {
                        if digit_ch.is_ascii_digit() {
                            num_str.push(digit_ch);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if let Ok(index) = num_str.parse::<usize>() {
                        if index == 0 {
                            return Err("Placeholder indices must start at 1 (found €0)".into());
                        }
                        required_args.insert(index);
                        result.push_str(&format!("{{ARG_{}}}", index));
                    } else {
                        return Err(format!("Invalid placeholder number: €{}", num_str));
                    }
                } else {
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }
    
    Ok(result)
}

fn generate_executable_source(api_key: Option<&str>, model: &str, pseudocode: &str, required_args: &[usize]) -> String {
    let escaped_model = escape_rust_string(model);
    let escaped_code = escape_rust_string(pseudocode);
    
    // Generate encrypted key and XOR key if API key is provided
    let (encrypted_key_bytes, xor_key_bytes) = if let Some(key) = api_key {
        let xor_key = generate_xor_key();
        let encrypted = xor_encrypt(key, &xor_key);
        (encrypted, xor_key)
    } else {
        (Vec::new(), Vec::new())
    };
    
    let encrypted_key_str = encrypted_key_bytes.iter()
        .map(|b| format!("{}", b))
        .collect::<Vec<_>>()
        .join(", ");
    
    let xor_key_str = xor_key_bytes.iter()
        .map(|b| format!("{}", b))
        .collect::<Vec<_>>()
        .join(", ");
    
    let has_embedded_key = api_key.is_some();
    
    let max_arg = required_args.iter().max().copied().unwrap_or(0);
    let arg_reading_code = if max_arg > 0 {
        let substitutions = required_args.iter().map(|&i| {
            format!("    pseudocode = pseudocode.replace(\"{{ARG_{}}}\", &lines[{}]);", i, i - 1)
        }).collect::<Vec<_>>().join("\n");
        
        format!(r#"
    // Check if stdin is available
    use std::io::{{IsTerminal, BufRead}};
    if io::stdin().is_terminal() {{
        eprintln!("Error: This program expects {} line(s) from stdin.\nUsage: echo 'value' | €0 or cat file | €0");
        std::process::exit(2);
    }}
    
    // Read arguments from stdin
    let stdin = io::stdin();
    let mut lines: Vec<String> = Vec::new();
    for line in stdin.lock().lines() {{
        lines.push(line.expect("Failed to read line from stdin"));
        if lines.len() >= {} {{
            break;
        }}
    }}
    
    if lines.len() < {} {{
        eprintln!("Error: Expected {} arguments from stdin, got {{}}\nUsage: Pipe {} lines into this program, one per line.", lines.len());
        std::process::exit(2);
    }}
    
    // Substitute placeholders in pseudocode
    let mut pseudocode = pseudocode.to_string();
{}
"#, max_arg, max_arg, max_arg, max_arg, max_arg, substitutions)
    } else {
        String::new()
    };
    
    let pseudocode_var = if max_arg > 0 { "&pseudocode" } else { "pseudocode" };
    
    let code = format!(
r###"use std::io::{{self, Write}};

#[tokio::main]
async fn main() {{
    // Try environment variable first, then fall back to embedded key
    let api_key = if let Ok(env_key) = std::env::var("OPENAI_API_KEY") {{
        env_key
    }} else if {} {{
        // Decrypt embedded key using XOR
        let encrypted: Vec<u8> = vec![{}];
        let xor_key: Vec<u8> = vec![{}];
        let decrypted: Vec<u8> = encrypted
            .iter()
            .enumerate()
            .map(|(i, &b)| b ^ xor_key[i % xor_key.len()])
            .collect();
        String::from_utf8(decrypted).expect("Invalid API key")
    }} else {{
        eprintln!("Error: No API key found. Set OPENAI_API_KEY environment variable.");
        std::process::exit(1);
    }};
    
    let model = "{}";
    let pseudocode = "{}";{}
    
    if let Err(e) = run_openai_stream(&api_key, model, {}).await {{
        eprintln!("Error: {{}}", e);
        std::process::exit(1);
    }}
}}

async fn run_openai_stream(api_key: &str, model: &str, pseudocode: &str) -> Result<(), Box<dyn std::error::Error>> {{
    let prompt = "You are an assistant that acts as if it were a program written in a language called 'matthiashihic'. This language allows every string to become a new string. Don't take it too literally, and ignore everything that doesn't make sense. If the user asks you to 'say' or 'make' something, for instance, just print it. Answer the code statement as if you had computed them. Do not reply with anything but the result.";
    
    let client = reqwest::Client::new();
    let request_body = serde_json::json!({{
        "model": model,
        "messages": [
            {{
                "role": "system",
                "content": prompt
            }},
            {{
                "role": "user",
                "content": pseudocode
            }}
        ],
        "stream": true
    }});
    
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {{}}", api_key))
        .json(&request_body)
        .send()
        .await?;
    
    if !response.status().is_success() {{
        let status = response.status();
        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("OpenAI API error ({{}}): {{}}", status, error_text).into());
    }}
    
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    
    while let Some(chunk_result) = stream.next().await {{
        let chunk = chunk_result?;
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);
        
        while let Some(newline_pos) = buffer.find('\n') {{
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();
            
            if line.starts_with("data: ") {{
                let data = &line[6..];
                if data.trim() == "[DONE]" {{
                    break;
                }}
                
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {{
                    if let Some(choices) = parsed["choices"].as_array() {{
                        if let Some(choice) = choices.first() {{
                            if let Some(content) = choice["delta"]["content"].as_str() {{
                                if !content.is_empty() {{
                                    print!("{{}}", content);
                                    io::stdout().flush()?;
                                }}
                            }}
                        }}
                    }}
                }}
            }}
        }}
    }}
    
    println!();
    Ok(())
}}
"###, has_embedded_key, encrypted_key_str, xor_key_str, escaped_model, escaped_code, arg_reading_code, pseudocode_var);
    code
}

fn make_temp_project_dir(prefix: &str) -> std::path::PathBuf {
    let mut p = env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    p.push(format!("{}-{}-{}", prefix, pid, nanos));
    p
}

fn create_cargo_project(project_dir: &std::path::Path, rust_source: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create project structure
    fs::create_dir_all(project_dir)?;
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)?;
    
    // Write main.rs
    fs::write(src_dir.join("main.rs"), rust_source)?;
    
    // Write Cargo.toml
    let cargo_toml = r#"[package]
name = "matthiashihic_exec"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = { version = "0.12", features = ["json", "stream"] }
serde_json = "1.0"
tokio = { version = "1", features = ["full"] }
futures-util = "0.3"
"#;
    fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;
    
    Ok(())
}



fn parse_matthiashihic(contents: &str) -> Result<(String, Vec<usize>), String> {
    // Split into lines but preserve order.
    let lines: Vec<&str> = contents.lines().collect();
    let mut required_args = std::collections::HashSet::<usize>::new();

    // Find first non-empty line
    let mut idx = 0usize;
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx >= lines.len() {
        return Err("Empty file; expected 'hihi!' header".into());
    }
    if lines[idx].trim() != "hihi!" {
        return Err("First non-empty line must be exactly: hihi!".into());
    }
    idx += 1;

    let mut code_lines = Vec::<String>::new();
    let mut terminator_found = false;
    while idx < lines.len() {
        let line = lines[idx];
        let t = line.trim();
        if t.is_empty() {
            idx += 1;
            continue;
        }
        if t == "eat that java!" {
            terminator_found = true;
            break;
        }
        // Parse a quoted string line: must start with " and end with "
        let trimmed = line.trim_start();
        if !trimmed.starts_with('\"') {
            return Err(format!(
                "Only quoted string statements allowed. Error at line {}: {}",
                idx + 1,
                line
            ));
        }
        // parse contents until unescaped closing quote
        let mut inner = String::new();
        let mut escaped = false;
        let mut found_closing_quote = false;
        let mut char_indices = trimmed.char_indices().skip(1); // skip opening quote
        
        while let Some((pos, ch)) = char_indices.next() {
            if escaped {
                // simple escapes: \n, \t, \r, \\, \"
                let mapped = match ch {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '"' => '"',
                    other => other, // unknown escape -> take literally
                };
                inner.push(mapped);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                // done; ensure the rest are whitespace
                let rest = &trimmed[pos + ch.len_utf8()..];
                if rest.trim().is_empty() {
                    found_closing_quote = true;
                    // Process the string for €index placeholders and €€index escaping
                    let processed = process_placeholders(&inner, &mut required_args)?;
                    code_lines.push(processed);
                    break;
                } else {
                    return Err(format!(
                        "Trailing characters after closing quote at line {}: {}",
                        idx + 1,
                        rest
                    ));
                }
            } else {
                inner.push(ch);
            }
        }
        // If the inner string didn't get closed (we exited loop), try to detect that:
        if !found_closing_quote {
            // It means we didn't find a closing quote properly
            return Err(format!(
                "Missing closing quote for string starting at line {}: {}",
                idx + 1,
                line
            ));
        }
        idx += 1;
    }

    if !terminator_found {
        return Err("Missing terminator line: eat that java!".into());
    }

    let mut args_vec: Vec<usize> = required_args.into_iter().collect();
    args_vec.sort();
    Ok((code_lines.join("\n"), args_vec))
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let prog = args.get(0).map(|s| s.as_str()).unwrap_or("matthiashihic");
    if args.len() < 2 {
        usage_and_exit(prog);
    }

    let mut src_path: Option<String> = None;
    let mut api_key: Option<String> = None;
    let mut model: String = "gpt-4".to_string();
    let mut out_path: Option<std::path::PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--api-key" => {
                if i + 1 >= args.len() {
                    eprintln!("--api-key requires an argument");
                    usage_and_exit(prog);
                }
                api_key = Some(args[i + 1].clone());
                i += 2;
            }
            "--model" => {
                if i + 1 >= args.len() {
                    eprintln!("--model requires an argument");
                    usage_and_exit(prog);
                }
                model = args[i + 1].clone();
                i += 2;
            }
            "-o" => {
                if i + 1 >= args.len() {
                    eprintln!("-o requires an argument");
                    usage_and_exit(prog);
                }
                out_path = Some(std::path::PathBuf::from(args[i + 1].clone()));
                i += 2;
            }
            s if s.starts_with('-') => {
                eprintln!("Unknown flag: {}", s);
                usage_and_exit(prog);
            }
            s => {
                if src_path.is_some() {
                    eprintln!("Multiple source files not supported");
                    usage_and_exit(prog);
                }
                src_path = Some(s.to_string());
                i += 1;
            }
        }
    }

    let src_path = match src_path {
        Some(p) => p,
        None => {
            eprintln!("No source file specified");
            usage_and_exit(prog);
        }
    };
    
    // API key is now optional - can be provided at compile time or runtime via env var
    if api_key.is_none() {
        eprintln!("Note: No --api-key provided. Compiled program will require OPENAI_API_KEY environment variable.");
    }

    let src_path_buf = std::path::PathBuf::from(&src_path);
    if !src_path_buf.exists() {
        eprintln!("Source file does not exist: {}", src_path);
        std::process::exit(1);
    }

    // Default output name: source filename without extension
    let out_path = match out_path {
        Some(p) => p,
        None => {
            let stem = src_path_buf
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("a.out");
            std::path::PathBuf::from(stem)
        }
    };

    let src_contents = match fs::read_to_string(&src_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to read {}: {}", src_path, e);
            std::process::exit(1);
        }
    };

    let (pseudocode, required_args) = match parse_matthiashihic(&src_contents) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(2);
        }
    };

    // Generate Rust source code for the executable
    let rust_src = generate_executable_source(api_key.as_deref(), &model, &pseudocode, &required_args);

    // Create temporary Cargo project
    let temp_project = make_temp_project_dir("matthiashihic");
    if let Err(e) = create_cargo_project(&temp_project, &rust_src) {
        eprintln!("Failed to create temporary Cargo project: {}", e);
        std::process::exit(1);
    }

    // Compile with cargo
    let out_str = out_path.to_string_lossy();
    eprintln!(
        "Compiling {} -> {} using cargo ...",
        src_path,
        out_str
    );
    let status = std::process::Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(temp_project.join("Cargo.toml"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    let compiled_binary = temp_project.join("target").join("release").join("matthiashihic_exec");

    match status {
        Ok(s) if s.success() => {
            // Copy compiled binary to output location
            if let Err(e) = fs::copy(&compiled_binary, &out_path) {
                eprintln!("Failed to copy binary to {}: {}", out_str, e);
                let _ = fs::remove_dir_all(&temp_project);
                std::process::exit(1);
            }
            
            // Make sure executable bit is set (on Unix)
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = fs::metadata(&out_path) {
                    let mut perm = meta.permissions();
                    perm.set_mode(0o755);
                    let _ = fs::set_permissions(&out_path, perm);
                }
            }
            
            // Clean up temp project
            let _ = fs::remove_dir_all(&temp_project);
            
            println!("Built executable: {}", out_str);
            std::process::exit(0);
        }
        Ok(s) => {
            eprintln!("Compiler exited with status: {}", s);
            let _ = fs::remove_dir_all(&temp_project);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Failed to spawn cargo: {}", e);
            let _ = fs::remove_dir_all(&temp_project);
            std::process::exit(1);
        }
    }
}