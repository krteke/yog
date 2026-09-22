#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextInput {
    value: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            cursor: value.len(),
            value,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn begin(&mut self) {
        self.cursor = self.value.len();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    pub fn insert(&mut self, value: char) {
        self.value.insert(self.cursor, value);
        self.cursor += value.len_utf8();
    }

    pub fn backspace(&mut self) {
        let Some((index, _)) = self.value[..self.cursor].char_indices().next_back() else {
            return;
        };
        self.value.remove(index);
        self.cursor = index;
    }

    pub fn delete(&mut self) {
        if self.cursor < self.value.len() {
            self.value.remove(self.cursor);
        }
    }

    pub fn move_left(&mut self) {
        if let Some((index, _)) = self.value[..self.cursor].char_indices().next_back() {
            self.cursor = index;
        }
    }

    pub fn move_right(&mut self) {
        if let Some(value) = self.value[self.cursor..].chars().next() {
            self.cursor += value.len_utf8();
        }
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.value.len();
    }

    pub fn display(&self, editing: bool) -> String {
        if !editing {
            return if self.value.is_empty() {
                "—".to_owned()
            } else {
                self.value.clone()
            };
        }

        let mut value = self.value.clone();
        value.insert(self.cursor, '│');
        value
    }
}
