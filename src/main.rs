#![feature(deque_extras)]
#![feature(box_syntax)]
#![feature(plugin)]
#![plugin(peg_syntax_ext)]

extern crate glob;
extern crate regex;

use std::collections::HashMap;
use std::fs::File;
use std::io::{stdout, Read, Write};
use std::env;
use std::process;

use self::directory_stack::DirectoryStack;
use self::input_editor::readln;
use self::peg::{parse, Pipeline};
use self::variables::Variables;
use self::history::History;
use self::flow_control::{FlowControl, is_flow_control_command, Statement};
use self::status::{SUCCESS, NO_SUCH_COMMAND};
use self::function::Function;
use self::pipe::execute_pipeline;

pub mod pipe;
pub mod directory_stack;
pub mod to_num;
pub mod input_editor;
pub mod peg;
pub mod variables;
pub mod history;
pub mod flow_control;
pub mod status;
pub mod function;

/// This struct will contain all of the data structures related to this
/// instance of the shell.
pub struct Shell {
    variables: Variables,
    flow_control: FlowControl,
    directory_stack: DirectoryStack,
    history: History,
    functions: HashMap<String, Function>
}

impl Shell {
    /// Panics if DirectoryStack construction fails
    pub fn new() -> Self {
        let mut new_shell = Shell {
            variables: Variables::new(),
            flow_control: FlowControl::new(),
            directory_stack: DirectoryStack::new().expect(""),
            history: History::new(),
            functions: HashMap::new()
        };
        new_shell.initialize_default_variables();
        new_shell.evaluate_init_file();
        return new_shell;
    }

    /// This function will initialize the default variables used by the shell. This function will
    /// be called before evaluating the init
    fn initialize_default_variables(&mut self) {
        self.variables.set_var("DIRECTORY_STACK_SIZE", "1000");
        self.variables.set_var("HISTORY_SIZE", "1000");
        self.variables.set_var("HISTORY_FILE_ENABLED", "1");
        self.variables.set_var("HISTORY_FILE_SIZE", "1000");
        self.variables.set_var("PROMPT", "ion:$PWD# ");

        {   // Initialize the HISTORY_FILE variable
            let mut history_path = std::env::home_dir().unwrap();
            history_path.push(".ion_history");
            self.variables.set_var("HISTORY_FILE", history_path.to_str().unwrap_or("?"));
        }

        // Initialize the PWD (Present Working Directory) variable
        match std::env::current_dir() {
            Ok(path) => self.variables.set_var("PWD", path.to_str().unwrap_or("?")),
            Err(_)   => self.variables.set_var("PWD", "?")
        }

        // Initialize the HOME variable
        match std::env::home_dir() {
            Some(path) => self.variables.set_var("HOME", path.to_str().unwrap_or("?")),
            None       => self.variables.set_var("HOME", "?")
        }
    }

    /// This functional will update variables that need to be kept consistent with each iteration
    /// of the prompt. In example, the PWD variable needs to be updated to reflect changes to the
    /// the current working directory.
    fn update_variables(&mut self) {
        // Update the PWD (Present Working Directory) variable if the current working directory has
        // been updated.
        match std::env::current_dir() {
            Ok(path) => {
                let pwd = self.variables.expand_string("$PWD");
                let pwd = pwd.as_str();
                let current_dir = path.to_str().unwrap_or("?");
                if pwd != current_dir {
                    self.variables.set_var("OLDPWD", pwd);
                    self.variables.set_var("PWD", current_dir);
                }
            },
            Err(_)   => self.variables.set_var("PWD", "?")
        }

    }

    /// Evaluates the source init file in the user's home directory. If the file does not exist,
    /// the file will be created.
    fn evaluate_init_file(&mut self) {
        let commands = &Command::map();
        let mut source_file = std::env::home_dir().unwrap(); // Obtain home directory
        source_file.push(".ionrc");                          // Location of ion init file

        if let Ok(mut file) = File::open(source_file.clone()) {
            let mut command_list = String::new();
            if let Err(message) = file.read_to_string(&mut command_list) {
                println!("{}: Failed to read {:?}", message, source_file.clone());
            } else {
                self.on_command(&command_list, commands);
            }
        } else {
            if let Err(message) = File::create(source_file) {
                println!("{}", message);
            }
        }
    }

    pub fn print_prompt(&self) {
        self.print_prompt_prefix();
        match self.flow_control.get_last_block().statement {
            Statement::For(_, _) => self.print_for_prompt(),
            Statement::Function(_, _) => self.print_function_prompt(),
            _ => self.print_default_prompt(),
        }
        if let Err(message) = stdout().flush() {
            println!("{}: failed to flush prompt to stdout", message);
        }

    }

    // TODO eventually this thing should be gone
    fn print_prompt_prefix(&self) {
        let prompt_prefix = self.flow_control.blocks.iter().fold(String::new(), |acc, block| {
            acc +
            if let Statement::If(value) = block.statement {
                if value {
                    "+ "
                } else {
                    "- "
                }
            } else {
                ""
            }
        });
        print!("{}", prompt_prefix);
    }

    fn print_for_prompt(&self) {
        print!("for> ");
    }

    fn print_function_prompt(&self) {
        print!("fn> ");
    }

    fn print_default_prompt(&self) {
        print!("{}", self.variables.expand_string(&self.variables.expand_string("$PROMPT")));
    }

    fn on_command(&mut self, command_string: &str, commands: &HashMap<&str, Command>) {
        self.history.add(command_string.to_string(), &self.variables);

        let mut pipelines = parse(command_string);

        // Execute commands
        for pipeline in pipelines.drain(..) {
            if self.flow_control.get_last_block().collecting {
                // TODO move this logic into "end" command
                if pipeline.jobs[0].command == "end" {
                    let block_jobs: Vec<Pipeline> = self.flow_control
                                                   .blocks
                                                   .last_mut()
                                                   .unwrap()
                                                   .pipelines
                                                   .drain(..)
                                                   .collect();
                    match self.flow_control.get_last_block().statement.clone() {
                        Statement::For(ref var, ref vals) => {
                            let variable = var.clone();
                            let values = vals.clone();
                            for value in values {
                                self.variables.set_var(&variable, &value);
                                for pipeline in block_jobs.iter() {
                                    self.run_pipeline(&pipeline, commands);
                                }
                            }
                        },
                        Statement::Function(ref name, ref args) => {
                            self.functions.insert(name.clone(), Function { name: name.clone(), pipelines: block_jobs.clone(), args: args.clone() });
                        },
                        _ => {}
                    }
                    self.run_pipeline(&pipeline, commands); // run the end command to pop the current block
                } else {
                    self.flow_control.get_last_block_mut().pipelines.push(pipeline);
                }
            } else {
                if self.flow_control.skipping() && !is_flow_control_command(&pipeline.jobs[0].command) {
                    continue;
                }
                self.run_pipeline(&pipeline, commands);
            }
        }
    }

    fn run_pipeline(&mut self, pipeline: &Pipeline, commands: &HashMap<&str, Command>) -> Option<i32> {
        let mut pipeline = self.variables.expand_pipeline(pipeline);
        pipeline.expand_globs();
        let exit_status = if let Some(command) = commands.get(pipeline.jobs[0].command.as_str()) {
            Some((*command.main)(pipeline.jobs[0].args.as_slice(), self))
        } else if let Some(function) = self.functions.get(pipeline.jobs[0].command.as_str()).cloned() {
            if pipeline.jobs[0].args.len() - 1 != function.args.len() {
                println!("This function takes {} arguments, but you provided {}", function.args.len(), pipeline.jobs[0].args.len()-1);
                Some(NO_SUCH_COMMAND) // not sure if this is the right error code
            } else {
                let mut variables_backup: HashMap<&str, Option<String>> = HashMap::new();
                for (name, value) in function.args.iter().zip(pipeline.jobs[0].args.iter().skip(1)) {
                    variables_backup.insert(name, self.variables.get_var(name).cloned());
                    self.variables.set_var(name, value);
                }
                let mut return_value = None;
                for function_pipeline in function.pipelines.iter() {
                    return_value = self.run_pipeline(function_pipeline, commands)
                }
                for (name, value_option) in variables_backup.iter() {
                    match *value_option {
                        Some(ref value) => self.variables.set_var(name, value),
                        None => {
                            self.variables.unset_var(name);
                        }
                    }
                }
                return_value
            }
        } else {
            Some(execute_pipeline(pipeline))
        };
        if let Some(code) = exit_status {
            self.variables.set_var("?", &code.to_string());
            self.history.previous_status = code;
        }
        exit_status
    }

    /// Evaluates the given file and returns 'SUCCESS' if it succeeds.
    fn source_command(&mut self, arguments: &[String]) -> i32 {
        let commands = Command::map();
        match arguments.iter().skip(1).next() {
            Some(argument) => {
                if let Ok(mut file) = File::open(&argument) {
                    let mut command_list = String::new();
                    if let Err(message) = file.read_to_string(&mut command_list) {
                        println!("{}: Failed to read {}", message, argument);
                        return status::FAILURE;
                    } else {
                        self.on_command(&command_list, &commands);
                        return status::SUCCESS;
                    }
                } else {
                    println!("Failed to open {}", argument);
                    return status::FAILURE;
                }
            },
            None => {
                self.evaluate_init_file();
                return status::SUCCESS;
            },
        }
    }
}

/// Structure which represents a Terminal's command.
/// This command structure contains a name, and the code which run the
/// functionnality associated to this one, with zero, one or several argument(s).
/// # Example
/// ```
/// let my_command = Command {
///     name: "my_command",
///     help: "Describe what my_command does followed by a newline showing usage",
///     main: box|args: &[String], &mut Shell| -> i32 {
///         println!("Say 'hello' to my command! :-D");
///     }
/// }
/// ```
pub struct Command {
    pub name: &'static str,
    pub help: &'static str,
    pub main: Box<Fn(&[String], &mut Shell) -> i32>,
}

impl Command {
    /// Return the map from command names to commands
    pub fn map() -> HashMap<&'static str, Self> {
        let mut commands: HashMap<&str, Self> = HashMap::new();

        commands.insert("cd",
                        Command {
                            name: "cd",
                            help: "Change the current directory\n    cd <path>",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.directory_stack.cd(args, &shell.variables)
                            },
                        });

        commands.insert("dirs",
                        Command {
                            name: "dirs",
                            help: "Display the current directory stack",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.directory_stack.dirs(args)
                            },
                        });

        commands.insert("exit",
                        Command {
                            name: "exit",
                            help: "To exit the curent session",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                if let Some(status) = args.get(1) {
                                    if let Ok(status) = status.parse::<i32>() {
                                        process::exit(status);
                                    }
                                }
                                process::exit(shell.history.previous_status);
                            },
                        });

        commands.insert("let",
                        Command {
                            name: "let",
                            help: "View, set or unset variables",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.variables.let_(args)
                            },
                        });

        commands.insert("read",
                        Command {
                            name: "read",
                            help: "Read some variables\n    read <variable>",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.variables.read(args)
                            },
                        });

        commands.insert("pushd",
                        Command {
                            name: "pushd",
                            help: "Push a directory to the stack",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.directory_stack.pushd(args, &shell.variables)
                            },
                        });

        commands.insert("popd",
                        Command {
                            name: "popd",
                            help: "Pop a directory from the stack",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.directory_stack.popd(args)
                            },
                        });

        commands.insert("history",
                        Command {
                            name: "history",
                            help: "Display a log of all commands previously executed",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.history.history(args)
                            },
                        });

        commands.insert("if",
                        Command {
                            name: "if",
                            help: "Conditionally execute code",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.flow_control.if_(args)
                            },
                        });

        commands.insert("else",
                        Command {
                            name: "else",
                            help: "Execute code if a previous condition was false",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.flow_control.else_(args)
                            },
                        });

        commands.insert("end",
                        Command {
                            name: "end",
                            help: "End a code block",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.flow_control.end(args)
                            },
                        });

        commands.insert("for",
                        Command {
                            name: "for",
                            help: "Iterate through a list",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.flow_control.for_(args)
                            },
                        });

        commands.insert("source",
                        Command {
                            name: "source",
                            help: "Evaluate the file following the command or re-initialize the init file",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.source_command(args)

                            },
                        });

        commands.insert("true",
                        Command {
                            name: "true",
                            help: "Do nothing, successfully",
                            main: box |_: &[String], _: &mut Shell| -> i32 {
                                status::SUCCESS
                            },
                        });

        commands.insert("false",
                        Command {
                            name: "false",
                            help: "Do nothing, unsuccessfully",
                            main: box |_: &[String], _: &mut Shell| -> i32 {
                                status::FAILURE
                            },
                        });

        commands.insert("fn",
                        Command {
                            name: "fn",
                            help: "Create a function",
                            main: box |args: &[String], shell: &mut Shell| -> i32 {
                                shell.flow_control.fn_(args)
                            },
                        });

        let command_helper: HashMap<&'static str, &'static str> = commands.iter()
                                                                          .map(|(k, v)| {
                                                                              (*k, v.help)
                                                                          })
                                                                          .collect();

        commands.insert("help",
                        Command {
                            name: "help",
                            help: "Display helpful information about a given command, or list \
                                   commands if none specified\n    help <command>",
                            main: box move |args: &[String], _: &mut Shell| -> i32 {
                                if let Some(command) = args.get(1) {
                                    if command_helper.contains_key(command.as_str()) {
                                        match command_helper.get(command.as_str()) {
                                            Some(help) => println!("{}", help),
                                            None => {
                                                println!("Command helper not found [run 'help']...")
                                            }
                                        }
                                    } else {
                                        println!("Command helper not found [run 'help']...");
                                    }
                                } else {
                                    for (command, _help) in command_helper.iter() {
                                        println!("{}", command);
                                    }
                                }
                                SUCCESS
                            },
                        });

        commands
    }
}

fn main() {
    let commands = Command::map();
    let mut shell = Shell::new();

    let mut dash_c = false;
    for arg in env::args().skip(1) {
        if arg == "-c" {
            dash_c = true;
        } else {
            if dash_c {
                shell.on_command(&arg, &commands);
            } else {
                match File::open(&arg) {
                    Ok(mut file) => {
                        let mut command_list = String::new();
                        match file.read_to_string(&mut command_list) {
                            Ok(_) => shell.on_command(&command_list, &commands),
                            Err(err) => println!("ion: failed to read {}: {}", arg, err)
                        }
                    },
                    Err(err) => println!("ion: failed to open {}: {}", arg, err)
                }
            }

            // Exit with the previous command's exit status.
            process::exit(shell.history.previous_status);
        }
    }

    shell.print_prompt();
    while let Some(command) = readln() {
        let command = command.trim();
        if !command.is_empty() {
            shell.on_command(command, &commands);
        }
        shell.update_variables();
        shell.print_prompt();
    }

    // Exit with the previous command's exit status.
    process::exit(shell.history.previous_status);
}
