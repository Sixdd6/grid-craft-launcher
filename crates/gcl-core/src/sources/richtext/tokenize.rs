//! The HTML tokenizer both converters read: tags, attributes, and entities.

#[cfg(test)]
mod tests;

/// Whether `chars` holds `pat` at `at`.
pub(super) fn starts_with(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(offset, c)| chars.get(at + offset) == Some(&c))
}

/// One piece of an HTML document: a text run or a tag.
#[derive(Debug)]
pub(super) enum Token {
    Text(String),
    Tag {
        name: String,
        close: bool,
        /// `<br/>` and friends: the tag opens no subtree.
        self_closing: bool,
        attrs: String,
    },
}

/// Splits HTML into text runs and tags. Comments and doctypes are dropped.
pub(super) fn tokenize(html: &str) -> Vec<Token> {
    let chars: Vec<char> = html.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '<' {
            text.push(chars[i]);
            i += 1;
            continue;
        }
        if starts_with(&chars, i, "<!--") {
            i = find_from(&chars, i, "-->").map_or(chars.len(), |end| end + 3);
            continue;
        }
        let Some(end) = tag_end(&chars, i) else {
            // A stray `<` with no `>` after it is text, not a tag.
            text.push(chars[i]);
            i += 1;
            continue;
        };
        if !text.is_empty() {
            out.push(Token::Text(std::mem::take(&mut text)));
        }
        let inner: String = chars[i + 1..end].iter().collect();
        if !inner.starts_with('!') {
            out.push(parse_tag(&inner));
        }
        i = end + 1;
    }
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    out
}

/// Index of the `>` closing the tag that starts at `at`, quotes respected.
fn tag_end(chars: &[char], at: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, c) in chars.iter().enumerate().skip(at + 1) {
        match quote {
            Some(q) if *c == q => quote = None,
            Some(_) => {}
            None if *c == '"' || *c == '\'' => quote = Some(*c),
            None if *c == '>' => return Some(i),
            None if *c == '<' => return None,
            None => {}
        }
    }
    None
}

/// Index of the first `pat` at or after `at`.
fn find_from(chars: &[char], at: usize, pat: &str) -> Option<usize> {
    (at..chars.len()).find(|i| starts_with(chars, *i, pat))
}

/// Splits a tag's inner text into its name, its attributes, and whether it closes itself.
fn parse_tag(inner: &str) -> Token {
    let inner = inner.trim();
    let self_closing = inner.ends_with('/');
    let inner = inner.trim_end_matches('/').trim_end();
    let close = inner.starts_with('/');
    let body = inner.trim_start_matches('/').trim_start();
    let split = body.find(|c: char| c.is_whitespace()).unwrap_or(body.len());
    Token::Tag {
        name: body[..split].to_ascii_lowercase(),
        close,
        self_closing,
        attrs: body[split..].trim().to_string(),
    }
}

/// The value of `key` in an attribute string, quoted or bare, or an empty string.
///
/// The string is read as name/value pairs, so a value that mentions `key` (a `title` with
/// `href=` in it, say) is never mistaken for the attribute itself.
pub(super) fn attr(attrs: &str, key: &str) -> String {
    for (name, value) in attr_pairs(attrs) {
        if name.eq_ignore_ascii_case(key) {
            return value;
        }
    }
    String::new()
}

/// Every `name="value"` pair in an attribute string, quotes respected.
fn attr_pairs(attrs: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = attrs.chars().collect();
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() || chars[i] == '=' {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if chars.get(i) != Some(&'=') {
            // A valueless attribute, such as `disabled`.
            pairs.push((name, String::new()));
            continue;
        }
        i += 1;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let value = match chars.get(i) {
            Some(quote @ ('"' | '\'')) => {
                let quote = *quote;
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != quote {
                    i += 1;
                }
                let value: String = chars[start..i].iter().collect();
                i += 1;
                value
            }
            _ => {
                let start = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[start..i].iter().collect()
            }
        };
        pairs.push((name, value.trim().to_string()));
    }
    pairs
}

/// Longest entity this decoder will look at, `&` and `;` included.
const MAX_ENTITY: usize = 12;

/// Replaces the HTML entities a description actually carries: numeric ones and a short
/// named list. Anything else is left as it was written.
pub(super) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = (i + 1..chars.len().min(i + MAX_ENTITY)).find(|j| chars[*j] == ';');
        match end.and_then(|end| entity(&chars[i + 1..end])) {
            Some(text) => {
                out.push_str(&text);
                i = end.unwrap_or(i) + 1;
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

/// The text an entity body (between `&` and `;`) stands for, or `None`.
fn entity(body: &[char]) -> Option<String> {
    let name: String = body.iter().collect();
    if let Some(number) = name.strip_prefix('#') {
        let value = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(value).map(String::from);
    }
    let text = match name.to_ascii_lowercase().as_str() {
        "nbsp" => " ",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "amp" => "&",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "rsquo" => "’",
        "lsquo" => "‘",
        "ldquo" => "“",
        "rdquo" => "”",
        "middot" => "·",
        "copy" => "©",
        "trade" => "™",
        "reg" => "®",
        _ => return None,
    };
    Some(text.to_string())
}
