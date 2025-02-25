use crate::builtins;
use crate::helpers::{Fd, Shell};
use crate::parser::{Cmd, Simple};
use os_pipe::{pipe, PipeReader, PipeWriter};
use std::cell::RefCell;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::rc::Rc;

// This is useful to keep track of what each command does with its STDs
#[derive(Debug)]
struct CmdMeta {
    stdin: Option<PipeReader>,
    stdout: Option<PipeWriter>,
}

impl CmdMeta {
    fn inherit() -> CmdMeta {
        CmdMeta {
            stdin: None,
            stdout: None,
        }
    }

    fn pipe_out(writer: PipeWriter) -> CmdMeta {
        CmdMeta {
            stdin: None,
            stdout: Some(writer),
        }
    }

    fn new_in(self, reader: PipeReader) -> CmdMeta {
        CmdMeta {
            stdin: Some(reader),
            stdout: self.stdout,
        }
    }
}

pub struct Runner {
    shell: Rc<RefCell<Shell>>,
}

impl Runner {
    pub fn new(shell: Rc<RefCell<Shell>>) -> Runner {
        Runner { shell }
    }

    pub fn execute(&self, ast: Cmd, capture: bool) -> Option<String> {
        if capture {
            let (mut reader, writer) = pipe().unwrap();
            self.visit(ast, CmdMeta::pipe_out(writer));
            let mut output = String::new();
            reader.read_to_string(&mut output).unwrap();
            Some(output)
        } else {
            self.visit(ast, CmdMeta::inherit());
            None
        }
    }

    // Probably not ideal for all of these to return a bool,
    // but it works for now. Once I figure out what's non-ideal
    // about it, I'll fix it
    fn visit(&self, node: Cmd, stdio: CmdMeta) -> bool {
        match node {
            Cmd::Simple(simple) => self.visit_simple(simple, stdio),
            Cmd::Pipeline(cmd0, cmd1) => self.visit_pipe(*cmd0, *cmd1, stdio),
            Cmd::And(cmd0, cmd1) => self.visit_and(*cmd0, *cmd1, stdio),
            Cmd::Or(cmd0, cmd1) => self.visit_or(*cmd0, *cmd1, stdio),
            Cmd::Not(cmd) => self.visit_not(*cmd, stdio),
            Cmd::Empty => true,
        }
    }

    fn visit_not(&self, cmd: Cmd, stdio: CmdMeta) -> bool {
        let result = self.visit(cmd, stdio);
        !result
    }

    fn visit_or(&self, left: Cmd, right: Cmd, stdio: CmdMeta) -> bool {
        let left = self.visit(left, CmdMeta::inherit());
        if left {
            left
        } else {
            self.visit(right, stdio)
        }
    }

    fn visit_and(&self, left: Cmd, right: Cmd, stdio: CmdMeta) -> bool {
        let left = self.visit(left, CmdMeta::inherit());
        if left {
            self.visit(right, stdio)
        } else {
            left
        }
    }

    // We create a pipe, pass the writing end to the left, and modify the stdio
    // to have its stdin be the reading end.
    fn visit_pipe(&self, left: Cmd, right: Cmd, stdio: CmdMeta) -> bool {
        let (reader, writer) = pipe().unwrap();
        self.visit(left, CmdMeta::pipe_out(writer));
        self.visit(right, stdio.new_in(reader))
    }

    fn visit_simple(&self, mut simple: Simple, stdio: CmdMeta) -> bool {
        self.reconcile_io(&mut simple, stdio);
        match &simple.cmd[..] {
            "exit" => builtins::exit(simple.args),
            "cd" => builtins::cd(simple.args),
            "set" => builtins::set(simple.args, &self.shell),
            "exec" => {
                if simple.args.is_empty() {
                    eprintln!("rush: exec: Not enough arguments");
                    false
                } else {
                    let err = Command::new(&simple.args[0]).args(&simple.args[1..]).exec();
                    eprintln!("rush: exec: {}", err);
                    false
                }
            }
            command => {
                let mut cmd = Command::new(command);
                cmd.args(&simple.args);

                if let Some(stdin) = simple.stdin.borrow_mut().get_stdin() {
                    cmd.stdin(stdin);
                } else {
                    return false;
                }
                if let Some(stdout) = simple.stdout.borrow_mut().get_stdout() {
                    cmd.stdout(stdout);
                } else {
                    return false;
                }
                if let Some(stderr) = simple.stdin.borrow_mut().get_stderr() {
                    cmd.stderr(stderr);
                } else {
                    return false;
                }
                if let Some(env) = simple.env {
                    cmd.envs(env);
                }

                match cmd.status() {
                    Ok(child) => child.success(),
                    Err(e) => {
                        eprintln!("rush: {}: {}", simple.cmd, e);
                        false
                    }
                }
            }
        }
    }

    // Takes the stdio and if stdio has priority, replaces stdout/stdin with it.
    fn reconcile_io(&self, simple: &mut Simple, stdio: CmdMeta) {
        if let Some(stdout) = stdio.stdout {
            if *simple.stdout.borrow() == Fd::Stdout {
                *simple.stdout.borrow_mut() = Fd::PipeOut(stdout);
            }
        }
        if let Some(stdin) = stdio.stdin {
            if *simple.stdin.borrow() == Fd::Stdin {
                *simple.stdin.borrow_mut() = Fd::PipeIn(stdin);
            }
        }
    }
}
// How do I test this module?
