//! Shell completion. Two names answer to the same binary, `deplyd` and the short
//! `dp`, and two options take values only the repository knows.

use std::io;

use clap::Command as ClapCommand;
use clap_complete::Shell;

/// Both names the binary answers to. The installer links the second to the first.
const NAMES: [&str; 2] = ["deplyd", "dp"];

pub fn emit(shell: Shell, command: &mut ClapCommand) {
    // --help and --version are added during the build, and completion should offer
    // them like any other flag.
    command.build();

    alias(shell);

    // PowerShell is hand-written because it is the only generator that can also ask
    // the binary for environments and authors, which is what the other half of
    // completion is for. The rest get names and flags.
    if shell == Shell::PowerShell {
        powershell(command);
        return;
    }

    for name in NAMES {
        clap_complete::generate(shell, command, name, &mut io::stdout());
    }
}

/// Only when nothing else owns the name: someone else's `dp` is not ours to take.
fn alias(shell: Shell) {
    match shell {
        Shell::Bash | Shell::Zsh => {
            println!("command -v dp >/dev/null 2>&1 || alias dp=deplyd");
        }
        Shell::Fish => {
            println!("command -v dp >/dev/null 2>&1; or alias dp deplyd");
        }
        Shell::PowerShell => {
            println!(
                "if (-not (Get-Command dp -ErrorAction SilentlyContinue)) {{ Set-Alias dp deplyd -Scope Global }}"
            );
        }
        _ => {}
    }
}

/// A PowerShell list literal: `'a', 'b'`.
fn list(values: impl IntoIterator<Item = String>) -> String {
    values
        .into_iter()
        .map(|value| format!("'{}'", value.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Name and description, for completion that shows what each one does. Objects
/// rather than nested arrays: PowerShell flattens an array literal of arrays, and
/// the pairs would come back as loose strings.
fn pairs(values: impl IntoIterator<Item = (String, String)>) -> String {
    values
        .into_iter()
        .map(|(name, about)| {
            format!(
                "    [pscustomobject]@{{ Name = '{}'; Description = '{}' }}",
                name.replace('\'', "''"),
                about.replace('\'', "''")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn powershell(command: &mut ClapCommand) {
    let subcommands = pairs(command.get_subcommands().filter(|sub| !sub.is_hide_set()).map(
        |sub| {
            (
                sub.get_name().to_string(),
                sub.get_about().map(|a| a.to_string()).unwrap_or_default(),
            )
        },
    ));

    let mut flags = Vec::new();
    let mut value_flags = Vec::new();
    for arg in command.get_arguments() {
        if arg.is_hide_set() {
            continue;
        }
        let about = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
        let takes_value = arg.get_num_args().is_none_or(|range| range.takes_values());

        for spelling in spellings(arg) {
            if takes_value {
                value_flags.push(spelling.clone());
            }
            flags.push((spelling, about.clone()));
        }
    }

    let environment_flags = list(flag_spellings(command, "environment"));
    let author_flags = list(flag_spellings(command, "author"));

    let shells = list(
        ["bash", "elvish", "fish", "powershell", "zsh"]
            .into_iter()
            .map(String::from),
    );

    print!(
        r#"
$script:DeplydCommands = @(
{subcommands}
)

$script:DeplydFlags = @(
{flags}
)

$script:DeplydValueFlags = @({value_flags})
$script:DeplydEnvironmentFlags = @({environment_flags})
$script:DeplydAuthorFlags = @({author_flags})

# Values the repository decides, so the binary is asked rather than guessed at. It
# reads and exits; a repository it cannot read simply completes nothing.
function script:DeplydAsk($exe, $what) {{
    try {{ & $exe complete $what 2>$null }} catch {{ @() }}
}}

function script:DeplydResults($values, $word) {{
    $values |
        Where-Object {{ $_ -and $_.StartsWith($word, [StringComparison]::OrdinalIgnoreCase) }} |
        ForEach-Object {{
            # A name with a space in it has to come back quoted or the shell splits it.
            $text = if ($_ -match '[\s'']') {{ "'" + $_.Replace("'", "''") + "'" }} else {{ $_ }}
            [System.Management.Automation.CompletionResult]::new(
                $text, $_, 'ParameterValue', $_)
        }}
}}

Register-ArgumentCompleter -Native -CommandName {names} -ScriptBlock {{
    param($wordToComplete, $commandAst, $cursorPosition)

    $tokens = @($commandAst.CommandElements | ForEach-Object {{ $_.ToString() }})
    $exe = if ($tokens.Count -ge 1) {{ $tokens[0] }} else {{ 'deplyd' }}
    $word = if ($null -eq $wordToComplete) {{ '' }} else {{ $wordToComplete }}

    # The word being typed is already a token, so the one before it is the last
    # finished token.
    $previous = if ($word -ne '' -and $tokens.Count -ge 2) {{
        $tokens[-2]
    }} elseif ($word -eq '' -and $tokens.Count -ge 1) {{
        $tokens[-1]
    }} else {{ '' }}

    if ($script:DeplydEnvironmentFlags -contains $previous) {{
        return script:DeplydResults (script:DeplydAsk $exe 'environments') $word
    }}
    if ($script:DeplydAuthorFlags -contains $previous) {{
        return script:DeplydResults (script:DeplydAsk $exe 'authors') $word
    }}
    # A flag still waiting for its value: nothing here can be suggested for it.
    if ($script:DeplydValueFlags -contains $previous) {{
        return @()
    }}

    if ($word.StartsWith('-')) {{
        return $script:DeplydFlags |
            Where-Object {{ $_.Name.StartsWith($word, [StringComparison]::OrdinalIgnoreCase) }} |
            ForEach-Object {{
                [System.Management.Automation.CompletionResult]::new(
                    $_.Name, $_.Name, 'ParameterName',
                    $(if ($_.Description) {{ $_.Description }} else {{ $_.Name }}))
            }}
    }}

    # Everything after the executable that is not a flag or a flag's value. The first
    # is the subcommand, and what may follow depends on which one it is.
    $words = @()
    for ($i = 1; $i -lt $tokens.Count; $i++) {{
        $token = $tokens[$i]
        if ($token.StartsWith('-')) {{
            if ($script:DeplydValueFlags -contains $token) {{ $i++ }}
            continue
        }}
        $words += $token
    }}
    # The word still being typed is one of those, and is not yet a choice made.
    # Written out rather than as a range: 0..-1 counts backwards in PowerShell.
    if ($word -ne '' -and $words.Count -gt 0) {{
        $words = @($words | Select-Object -First ($words.Count - 1))
    }}

    if ($words.Count -eq 0) {{
        return $script:DeplydCommands |
            Where-Object {{ $_.Name.StartsWith($word, [StringComparison]::OrdinalIgnoreCase) }} |
            ForEach-Object {{
                [System.Management.Automation.CompletionResult]::new(
                    $_.Name, $_.Name, 'ParameterValue',
                    $(if ($_.Description) {{ $_.Description }} else {{ $_.Name }}))
            }}
    }}

    # Subcommands shorten while they stay unambiguous, so match the same way.
    $subcommand = @($script:DeplydCommands |
        Where-Object {{ $_.Name.StartsWith($words[0], [StringComparison]::OrdinalIgnoreCase) }} |
        ForEach-Object {{ $_.Name }})
    $subcommand = if ($subcommand.Count -ge 1) {{ $subcommand[0] }} else {{ '' }}

    if ($words.Count -eq 1) {{
        switch ($subcommand) {{
            'remember'    {{ return script:DeplydResults @('author', 'environment', 'repo') $word }}
            'completions' {{ return script:DeplydResults @({shells}) $word }}
        }}
    }}
    if ($words.Count -eq 2 -and $subcommand -eq 'remember') {{
        switch ($words[1]) {{
            'environment' {{ return script:DeplydResults (script:DeplydAsk $exe 'environments') $word }}
            'author'      {{ return script:DeplydResults (script:DeplydAsk $exe 'authors') $word }}
        }}
    }}

    return @()
}}
"#,
        subcommands = subcommands,
        flags = pairs(flags),
        value_flags = list(value_flags),
        environment_flags = environment_flags,
        author_flags = author_flags,
        names = list(NAMES.iter().map(|name| name.to_string())),
        shells = shells,
    );
}

/// Every way one argument can be written: `--author` and `-A`.
fn spellings(arg: &clap::Arg) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(long) = arg.get_long() {
        out.push(format!("--{long}"));
    }
    if let Some(short) = arg.get_short() {
        out.push(format!("-{short}"));
    }
    out
}

fn flag_spellings(command: &ClapCommand, id: &str) -> Vec<String> {
    command
        .get_arguments()
        .find(|arg| arg.get_id() == id)
        .map(spellings)
        .unwrap_or_default()
}
