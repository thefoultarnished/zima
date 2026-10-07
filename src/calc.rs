//! A tiny calculator for `@calc`: + - * / % ^, parentheses, decimals, and "x" as multiply.
//! It's a hand-written arithmetic parser over numbers and operators only; nothing is executed.

pub fn evaluate(expr: &str) -> Option<f64> {
    let tokens = tokenize(expr)?;
    let mut parser = Parser { tokens, pos: 0 };
    let value = parser.expr()?;
    (parser.pos == parser.tokens.len() && value.is_finite()).then_some(value)
}

/// "50", "2.5", "0.3333333333" — no trailing zeros, at most 10 decimals.
pub fn format_number(value: f64) -> String {
    let rounded = (value * 1e10).round() / 1e10;
    let mut s = format!("{rounded:.10}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if s == "-0" { "0".into() } else { s }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Num(f64),
    Op(char),
    Open,
    Close,
}

fn tokenize(expr: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().filter(|c| **c != ',').collect();
                tokens.push(Token::Num(text.parse().ok()?));
            }
            '+' | '-' | '*' | '/' | '%' | '^' => {
                tokens.push(Token::Op(c));
                i += 1;
            }
            'x' | 'X' | '×' => {
                tokens.push(Token::Op('*'));
                i += 1;
            }
            '÷' => {
                tokens.push(Token::Op('/'));
                i += 1;
            }
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            _ => return None,
        }
    }
    (!tokens.is_empty()).then_some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek() {
            self.pos += 1;
            let rhs = self.term()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    // term := power (('*' | '/' | '%') power)*
    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        while let Some(Token::Op(op @ ('*' | '/' | '%'))) = self.peek() {
            self.pos += 1;
            let rhs = self.power()?;
            value = match op {
                '*' => value * rhs,
                '/' => value / rhs,
                _ => value % rhs,
            };
        }
        Some(value)
    }

    // power := unary ('^' power)?
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if let Some(Token::Op('^')) = self.peek() {
            self.pos += 1;
            return Some(base.powf(self.power()?));
        }
        Some(base)
    }

    // unary := '-' unary | atom
    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Op('-') => {
                self.pos += 1;
                Some(-self.unary()?)
            }
            Token::Op('+') => {
                self.pos += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Num(n) => {
                self.pos += 1;
                Some(n)
            }
            Token::Open => {
                self.pos += 1;
                let value = self.expr()?;
                (self.peek()? == Token::Close).then(|| self.pos += 1)?;
                Some(value)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates() {
        assert_eq!(evaluate("12*3.5 + 8"), Some(50.0));
        assert_eq!(evaluate("(1 + 2) * 3"), Some(9.0));
        assert_eq!(evaluate("2^3^2"), Some(512.0));
        assert_eq!(evaluate("-4 + 10 / 4"), Some(-1.5));
        assert_eq!(evaluate("3 x 4"), Some(12.0));
        assert_eq!(evaluate("1,200 + 1"), Some(1201.0));
        assert_eq!(evaluate("10 % 4"), Some(2.0));
        assert_eq!(evaluate("1 / 0"), None);
        assert_eq!(evaluate("2 +"), None);
        assert_eq!(evaluate("abc"), None);
    }

    #[test]
    fn formats() {
        assert_eq!(format_number(50.0), "50");
        assert_eq!(format_number(2.5), "2.5");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
    }
}
