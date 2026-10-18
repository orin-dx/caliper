use miette::Result;

pub fn cmd_completions(args: crate::CompletionsArgs) -> Result<()> {
    use clap::CommandFactory;
    clap_complete::generate(args.shell, &mut crate::Cli::command(), "caliper-docgen", &mut std::io::stdout());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    // ── SPEC-CLI-001a AC-014: completions with a recognized shell writes the
    // generated completion script and exits code 0, independent of any
    // source directory or extraction state. CompletionsArgs has no `src`
    // field at all — there is no way to even pass source directories to this
    // command, which is itself the structural proof that cmd_completions
    // never calls pipeline::extract. This tests the generation mechanism
    // cmd_completions calls internally, against a buffer instead of real
    // stdout (which a unit test can't cleanly intercept), for every shell
    // clap_complete::Shell recognizes.

    #[test]
    fn generates_a_non_empty_completion_script_for_every_recognized_shell() {
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::PowerShell,
            clap_complete::Shell::Elvish,
        ] {
            let mut buf = Vec::new();
            clap_complete::generate(shell, &mut crate::Cli::command(), "caliper-docgen", &mut buf);
            assert!(!buf.is_empty(), "expected a non-empty completion script for {shell:?}");
            let script = String::from_utf8(buf).expect("completion script should be valid UTF-8");
            assert!(
                script.contains("caliper-docgen"),
                "expected the completion script to reference the binary name, got {script:?}"
            );
        }
    }

    #[test]
    fn cmd_completions_itself_succeeds() {
        let code = cmd_completions(crate::CompletionsArgs { shell: clap_complete::Shell::Bash });
        assert!(code.is_ok(), "expected cmd_completions to succeed, got {code:?}");
    }
}
