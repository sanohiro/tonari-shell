// Only ordinary decimal loads and eager binary operators are compiled. A
// dynamic value, array or lazy branch goes through the full arithmetic parser.
// No assignment occurs until every operand has been resolved successfully.
use super::{Binary, Evaluation, Failure, Parser, Token, Variables, number, precedence};

#[derive(Debug, Clone)]
enum Op {
    Number(i64),
    Load(Box<str>),
    Binary(Binary),
}

#[derive(Debug, Clone)]
pub(crate) struct Program {
    ops: Box<[Op]>,
    target: Option<(Box<str>, Option<Binary>)>,
}

impl Program {
    pub(super) fn compile(source: &str) -> Option<Self> {
        if source.len() > 256 || source.contains(['$', '[', '\\', '"', '`']) {
            return None;
        }
        let mut parser = Parser::new(source);
        let mut ops = Vec::new();
        let mut operators = Vec::new();
        let mut target = None;
        let mut operand = true;
        let mut stack = 0usize;
        loop {
            parser.advance().ok()?;
            match parser.token {
                Token::End => break,
                Token::Op("(") if operand => {
                    operators.push("(");
                    continue;
                }
                Token::Number(s) if operand => ops.push(Op::Number(number(s).ok()?)),
                Token::Name(s) if operand && crate::word::is_identifier(s) => {
                    ops.push(Op::Load(s.into()))
                }
                Token::Op(")") if !operand => {
                    loop {
                        let op = operators.pop()?;
                        if op == "(" {
                            break;
                        }
                        ops.push(Op::Binary(Binary::parse(op)?));
                        stack = stack.checked_sub(1)?;
                    }
                    continue;
                }
                Token::Op(op) if !operand => {
                    let (prec, right) = precedence(op)?;
                    if matches!(op, "," | "?" | "||" | "&&") {
                        return None;
                    }
                    if prec == 2 {
                        if target.is_some() || ops.len() != 1 || !operators.is_empty() {
                            return None;
                        }
                        let Op::Load(name) = ops.pop()? else {
                            return None;
                        };
                        if !source
                            .trim_start()
                            .strip_prefix(name.as_ref())?
                            .trim_start()
                            .starts_with(op)
                        {
                            return None;
                        }
                        target = Some((
                            name,
                            if op == "=" {
                                None
                            } else {
                                Some(Binary::parse(op.strip_suffix('=')?)?)
                            },
                        ));
                        stack = 0;
                    } else {
                        while let Some(&top) = operators.last() {
                            if top == "(" {
                                break;
                            }
                            let (prev, _) = precedence(top)?;
                            if prev < prec || right && prev == prec {
                                break;
                            }
                            ops.push(Op::Binary(Binary::parse(operators.pop()?)?));
                            stack = stack.checked_sub(1)?;
                        }
                        operators.push(op);
                    }
                    operand = true;
                    continue;
                }
                _ => return None,
            }
            stack += 1;
            if stack > 16 {
                return None;
            }
            operand = false;
        }
        if operand {
            return None;
        }
        for op in operators.into_iter().rev() {
            if op == "(" {
                return None;
            }
            ops.push(Op::Binary(Binary::parse(op)?));
            stack = stack.checked_sub(1)?;
        }
        (stack == 1).then(|| Self {
            ops: ops.into_boxed_slice(),
            target,
        })
    }

    pub(super) fn run(
        &self,
        vars: &mut dyn Variables,
        permit_store: bool,
    ) -> Option<Evaluation<i64>> {
        if !permit_store && self.target.is_some() {
            return None;
        }
        let lhs = if let Some((name, op)) = &self.target
            && op.is_some()
        {
            vars.decimal(name)?
        } else {
            0
        };
        let mut stack = [0; 16];
        let mut top = 0usize;
        for op in &self.ops {
            let value = match op {
                Op::Number(n) => *n,
                Op::Load(name) => vars.decimal(name)?,
                Op::Binary(op) => {
                    top -= 2;
                    match op.apply(stack[top], stack[top + 1]) {
                        Ok(n) => n,
                        Err(e) => return Some(Err(e.into())),
                    }
                }
            };
            stack[top] = value;
            top += 1;
        }
        let mut value = stack[0];
        if let Some((name, op)) = &self.target {
            if let Some(op) = op {
                value = match op.apply(lhs, value) {
                    Ok(n) => n,
                    Err(e) => return Some(Err(e.into())),
                };
            }
            return Some(
                vars.set(name, value)
                    .map(|()| value)
                    .map_err(|e| Failure::write(name, e)),
            );
        }
        Some(Ok(value))
    }
}
