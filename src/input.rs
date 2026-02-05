pub struct CommandInput {
    buffer: String,
}

impl CommandInput {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    pub fn push_char(&mut self, ch: char) {
        self.buffer.push(ch);
    }

    pub fn on_backspace(&mut self) {
        self.buffer.pop();
    }

    pub fn on_enter(&mut self) -> Option<char> {
        let line = self.buffer.trim().to_string();
        self.buffer.clear();

        // Only handle the minimal `show X` command.
        parse_show_command(&line)
    }
}

fn parse_show_command(line: &str) -> Option<char> {
    let line = line.trim();
    if !line.starts_with("show ") {
        return None;
    }

    let rest = &line[5..];
    if rest.len() != 1 {
        return None;
    }

    let ch = rest.chars().next()?;
    if ch.is_ascii() {
        Some(ch)
    } else {
        None
    }
}
