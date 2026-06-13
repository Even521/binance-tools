#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngleMode {
    Deg,
    Rad,
}

#[derive(Clone, Debug)]
pub struct Calculator {
    expression: String,
    display: String,
    ans: f64,
    angle_mode: AngleMode,
    inverse: bool,
    just_evaluated: bool,
}

impl Default for Calculator {
    fn default() -> Self {
        Self {
            expression: String::new(),
            display: "0".to_string(),
            ans: 0.0,
            angle_mode: AngleMode::Deg,
            inverse: false,
            just_evaluated: false,
        }
    }
}

impl Calculator {
    pub fn press(&mut self, key: &str) -> Result<(), String> {
        match key {
            "Deg" => self.angle_mode = AngleMode::Deg,
            "Rad" => self.angle_mode = AngleMode::Rad,
            "Inv" => self.inverse = !self.inverse,
            "AC" => self.clear(),
            "=" => self.evaluate()?,
            "Ans" => self.append_value(&format_number(self.ans)),
            "π" => self.append_value("pi"),
            "e" => self.append_value("e"),
            "EXP" => self.append_raw("E"),
            "x!" => self.append_raw("!"),
            "%" => self.append_raw("%"),
            "x^y" => self.append_operator("^"),
            "√" => {
                if self.inverse {
                    self.append_function("square");
                } else {
                    self.append_function("sqrt");
                }
            }
            "sin" | "cos" | "tan" | "ln" | "log" => self.append_named_function(key),
            "+" | "−" | "-" | "×" | "÷" => self.append_operator(key),
            "(" | ")" | "." | "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                self.append_raw(key)
            }
            _ => return Err(format!("不支持的按键: {key}")),
        }
        if !matches!(key, "=") {
            self.sync_display();
        }
        Ok(())
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn expression(&self) -> &str {
        &self.expression
    }

    pub fn ans(&self) -> f64 {
        self.ans
    }

    pub fn angle_mode(&self) -> AngleMode {
        self.angle_mode
    }

    pub fn inverse(&self) -> bool {
        self.inverse
    }

    fn clear(&mut self) {
        self.expression.clear();
        self.display = "0".to_string();
        self.inverse = false;
        self.just_evaluated = false;
    }

    fn evaluate(&mut self) -> Result<(), String> {
        if self.expression.trim().is_empty() {
            self.display = "0".to_string();
            return Ok(());
        }
        let value = evaluate_expression(&self.expression, self.angle_mode, self.ans)?;
        self.ans = value;
        self.display = format_number(value);
        self.expression = self.display.clone();
        self.just_evaluated = true;
        self.inverse = false;
        Ok(())
    }

    fn append_named_function(&mut self, key: &str) {
        let name = match (self.inverse, key) {
            (true, "sin") => "asin",
            (true, "cos") => "acos",
            (true, "tan") => "atan",
            (true, "ln") => "exp",
            (true, "log") => "pow10",
            _ => key,
        };
        self.append_function(name);
    }

    fn append_function(&mut self, name: &str) {
        self.prepare_append_value();
        self.expression.push_str(name);
        self.expression.push('(');
        self.just_evaluated = false;
    }

    fn append_value(&mut self, value: &str) {
        self.prepare_append_value();
        self.expression.push_str(value);
        self.just_evaluated = false;
    }

    fn append_operator(&mut self, operator: &str) {
        let op = normalize_operator(operator);
        if self.expression.is_empty() {
            if op == "-" {
                self.expression.push('-');
            }
            return;
        }
        self.expression.push_str(op);
        self.just_evaluated = false;
    }

    fn append_raw(&mut self, value: &str) {
        if self.just_evaluated && starts_new_number(value) {
            self.expression.clear();
        }
        let value = match value {
            "−" => "-",
            "×" => "*",
            "÷" => "/",
            other => other,
        };
        self.expression.push_str(value);
        self.just_evaluated = false;
    }

    fn prepare_append_value(&mut self) {
        if self.just_evaluated {
            self.expression.clear();
        }
    }

    fn sync_display(&mut self) {
        self.display = if self.expression.is_empty() {
            "0".to_string()
        } else {
            pretty_expression(&self.expression)
        };
    }
}

pub fn evaluate_expression(
    expression: &str,
    angle_mode: AngleMode,
    ans: f64,
) -> Result<f64, String> {
    let mut parser = Parser::new(expression, angle_mode, ans);
    let value = parser.parse_expression()?;
    parser.skip_ws();
    if parser.is_end() {
        Ok(value)
    } else {
        Err("表达式后面还有无法识别的内容".to_string())
    }
}

fn normalize_operator(operator: &str) -> &str {
    match operator {
        "−" => "-",
        "×" => "*",
        "÷" => "/",
        other => other,
    }
}

fn starts_new_number(value: &str) -> bool {
    matches!(
        value,
        "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "." | "("
    )
}

fn pretty_expression(expression: &str) -> String {
    expression
        .replace('*', "×")
        .replace('/', "÷")
        .replace('-', "−")
        .replace("pi", "π")
}

fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value.abs() >= 1e12 || (value != 0.0 && value.abs() < 1e-9) {
        return format!("{value:.10e}");
    }
    let mut text = format!("{value:.10}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_string() } else { text }
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    angle_mode: AngleMode,
    ans: f64,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, angle_mode: AngleMode, ans: f64) -> Self {
        Self {
            input,
            pos: 0,
            angle_mode,
            ans,
        }
    }

    fn parse_expression(&mut self) -> Result<f64, String> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            if self.consume('+') {
                value += self.parse_term()?;
            } else if self.consume('-') {
                value -= self.parse_term()?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_ws();
            if self.consume('*') {
                value *= self.parse_power()?;
            } else if self.consume('/') {
                let rhs = self.parse_power()?;
                if rhs == 0.0 {
                    return Err("不能除以 0".to_string());
                }
                value /= rhs;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_power(&mut self) -> Result<f64, String> {
        let value = self.parse_unary()?;
        self.skip_ws();
        if self.consume('^') {
            Ok(value.powf(self.parse_power()?))
        } else {
            Ok(value)
        }
    }

    fn parse_unary(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.consume('+') {
            self.parse_unary()
        } else if self.consume('-') {
            Ok(-self.parse_unary()?)
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Result<f64, String> {
        let mut value = self.parse_primary()?;
        loop {
            self.skip_ws();
            if self.consume('!') {
                value = factorial(value)?;
            } else if self.consume('%') {
                value /= 100.0;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.consume('(') {
            let value = self.parse_expression()?;
            self.expect(')')?;
            return Ok(value);
        }

        if self
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
        {
            return self.parse_number();
        }

        if self.peek().is_some_and(|ch| ch.is_ascii_alphabetic()) {
            let ident = self.parse_ident();
            return match ident.as_str() {
                "pi" => Ok(std::f64::consts::PI),
                "e" => Ok(std::f64::consts::E),
                "Ans" | "ans" => Ok(self.ans),
                name => {
                    self.expect('(')?;
                    let value = self.parse_expression()?;
                    self.expect(')')?;
                    apply_function(name, value, self.angle_mode)
                }
            };
        }

        Err("缺少数字或函数".to_string())
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
        {
            self.advance();
        }
        if self.peek().is_some_and(|ch| ch == 'E' || ch == 'e') {
            self.advance();
            if self.peek().is_some_and(|ch| ch == '+' || ch == '-') {
                self.advance();
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.advance();
            }
        }
        self.input[start..self.pos]
            .parse::<f64>()
            .map_err(|_| "数字格式错误".to_string())
    }

    fn parse_ident(&mut self) -> String {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch.is_ascii_digit())
        {
            self.advance();
        }
        self.input[start..self.pos].to_string()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.advance();
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        self.skip_ws();
        if self.consume(expected) {
            Ok(())
        } else {
            Err(format!("缺少 `{expected}`"))
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        if let Some(ch) = self.peek() {
            self.pos += ch.len_utf8();
        }
    }

    fn is_end(&self) -> bool {
        self.pos >= self.input.len()
    }
}

fn apply_function(name: &str, value: f64, angle_mode: AngleMode) -> Result<f64, String> {
    let rad = match angle_mode {
        AngleMode::Deg => value.to_radians(),
        AngleMode::Rad => value,
    };
    let inverse_angle = |value: f64| match angle_mode {
        AngleMode::Deg => value.to_degrees(),
        AngleMode::Rad => value,
    };

    match name {
        "sin" => Ok(rad.sin()),
        "cos" => Ok(rad.cos()),
        "tan" => Ok(rad.tan()),
        "asin" => Ok(inverse_angle(value.asin())),
        "acos" => Ok(inverse_angle(value.acos())),
        "atan" => Ok(inverse_angle(value.atan())),
        "ln" => {
            if value <= 0.0 {
                Err("ln 的输入必须大于 0".to_string())
            } else {
                Ok(value.ln())
            }
        }
        "log" => {
            if value <= 0.0 {
                Err("log 的输入必须大于 0".to_string())
            } else {
                Ok(value.log10())
            }
        }
        "sqrt" => {
            if value < 0.0 {
                Err("平方根不能输入负数".to_string())
            } else {
                Ok(value.sqrt())
            }
        }
        "square" => Ok(value * value),
        "exp" => Ok(value.exp()),
        "pow10" => Ok(10_f64.powf(value)),
        _ => Err(format!("未知函数: {name}")),
    }
}

fn factorial(value: f64) -> Result<f64, String> {
    if value < 0.0 || value.fract().abs() > f64::EPSILON {
        return Err("阶乘只支持非负整数".to_string());
    }
    if value > 170.0 {
        return Err("阶乘结果太大".to_string());
    }
    let mut result = 1.0;
    for n in 1..=value as u64 {
        result *= n as f64;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expression: &str) -> f64 {
        evaluate_expression(expression, AngleMode::Deg, 0.0).unwrap()
    }

    #[test]
    fn evaluates_operator_precedence() {
        assert_eq!(eval("2+3*4"), 14.0);
        assert_eq!(eval("(2+3)*4"), 20.0);
        assert_eq!(eval("2^3^2"), 512.0);
    }

    #[test]
    fn evaluates_scientific_functions() {
        assert!((eval("sin(30)") - 0.5).abs() < 1e-10);
        assert!((eval("sqrt(9)") - 3.0).abs() < 1e-10);
        assert!((eval("log(100)") - 2.0).abs() < 1e-10);
        assert_eq!(eval("5!"), 120.0);
        assert_eq!(eval("50%"), 0.5);
    }

    #[test]
    fn supports_rad_mode() {
        let value = evaluate_expression("sin(pi/2)", AngleMode::Rad, 0.0).unwrap();
        assert!((value - 1.0).abs() < 1e-10);
    }

    #[test]
    fn supports_button_state_and_ans() {
        let mut calculator = Calculator::default();
        for key in ["7", "+", "8", "="] {
            calculator.press(key).unwrap();
        }
        assert_eq!(calculator.display(), "15");
        calculator.press("+").unwrap();
        calculator.press("Ans").unwrap();
        calculator.press("=").unwrap();
        assert_eq!(calculator.display(), "30");
    }

    #[test]
    fn rejects_invalid_input() {
        assert!(evaluate_expression("1/0", AngleMode::Deg, 0.0).is_err());
        assert!(evaluate_expression("sqrt(-1)", AngleMode::Deg, 0.0).is_err());
    }
}
