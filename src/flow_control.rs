use super::to_num::ToNum;
use super::peg::Job;
use super::status::{SUCCESS, FAILURE};

pub fn is_flow_control_command(command: &str) -> bool {
    command == "end" || command == "if" || command == "else"
}

#[derive(Clone)]
pub enum Statement {
    For(String, Vec<String>),
    Function(String),
    If(bool),
    Default,
}

pub struct CodeBlock {
    pub jobs: Vec<Job>,
}

pub struct Mode {
    pub value: bool,
}

pub struct FlowControl {
    pub collecting_block: bool,
    pub current_block: CodeBlock,
    pub statements: Vec<Statement>,
}

impl FlowControl {
    pub fn new() -> FlowControl {
        FlowControl {
            collecting_block: false,
            current_block: CodeBlock { jobs: vec![] },
            statements: vec![Statement::Default],
        }
    }

    pub fn skipping(&self) -> bool {
        self.statements.iter().any(|stat| match *stat {
            Statement::If(value) => !value,
            _ => false
        })
    }

    pub fn if_<I: IntoIterator>(&mut self, args: I) -> i32
        where I::Item: AsRef<str>
    {
        let mut args = args.into_iter(); // TODO why does the compiler want this to be mutable?
        let value;
        if let Some(left) = args.nth(1) {
            let left = left.as_ref();
            if let Some(cmp) = args.nth(0) {
                let cmp = cmp.as_ref();
                if let Some(right) = args.nth(0) {
                    let right = right.as_ref();
                    if cmp == "==" {
                        value = left == right;
                    } else if cmp == "!=" {
                        value = left != right;
                    } else if cmp == ">" {
                        value = left.to_num_signed() > right.to_num_signed();
                    } else if cmp == ">=" {
                        value = left.to_num_signed() >= right.to_num_signed();
                    } else if cmp == "<" {
                        value = left.to_num_signed() < right.to_num_signed();
                    } else if cmp == "<=" {
                        value = left.to_num_signed() <= right.to_num_signed();
                    } else {
                        println!("Unknown comparison: {}", cmp);
                        return FAILURE;
                    }
                } else {
                    println!("No right hand side");
                    return FAILURE;
                }
            } else {
                println!("No comparison operator");
                return FAILURE;
            }
        } else {
            println!("No left hand side");
            return FAILURE;
        }
        self.statements.push(Statement::If(value));
        SUCCESS
    }

    pub fn else_<I: IntoIterator>(&mut self, _: I) -> i32
        where I::Item: AsRef<str>
    {
        if let Some(&mut Statement::If(ref mut value)) = self.statements.last_mut() {
            *value = !*value;
            SUCCESS
        } else {
            println!("Syntax error: else found with no previous if");
            FAILURE
        }
    }

    pub fn end<I: IntoIterator>(&mut self, _: I) -> i32
        where I::Item: AsRef<str>
    {
        if self.statements.len() > 1{
            self.statements.pop();
            SUCCESS
        } else {
            println!("Syntax error: end found outside of a block");
            FAILURE
        }
    }

    pub fn for_<I: IntoIterator>(&mut self, args: I) -> i32
        where I::Item: AsRef<str>
    {
        let mut args = args.into_iter();
        if let Some(variable) = args.nth(1).map(|var| var.as_ref().to_string()) {
            if let Some(in_) = args.nth(0) {
                if in_.as_ref() != "in" {
                    println!("For loops must have 'in' as the second argument");
                    return FAILURE;
                }
            } else {
                println!("For loops must have 'in' as the second argument");
                return FAILURE;
            }
            let values: Vec<String> = args.map(|value| value.as_ref().to_string()).collect();
            self.statements.push(Statement::For(variable, values));
            self.collecting_block = true;
        } else {
            println!("For loops must have a variable name as the first argument");
            return FAILURE;
        }
        SUCCESS
    }

    pub fn fn_<I: IntoIterator>(&mut self, args: I) -> i32
        where I::Item: AsRef<str>
    {
        let mut args = args.into_iter();
        if let Some(name) = args.nth(1) {
            self.collecting_block = true;
            self.statements.push(Statement::Function(name.as_ref().to_string()));
        } else {
            println!("Functions must have the function name as the first argument");
            return FAILURE;
        }
        SUCCESS
    }
}
