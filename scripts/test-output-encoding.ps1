# Regression test for the output encoding of the `jira` CLI (src/output.rs).
#
# Windows PowerShell decodes what a native program writes to a pipe with the
# console output code page, so with a 936 (zh-CN) or 437 (en-US) console the
# UTF-8 `jira` prints would come out as mojibake. The CLI therefore encodes
# its output for PowerShell. This checks that Chinese text survives without
# any shell configuration, and that UTF-8 consumers are left alone.
#
# Needs no Jira server: a throwaway config file and --dry-run.
#
#   scripts/test-output-encoding.ps1 -Jira <jira.exe> -Work <scratch dir>
#
# ASCII only: Windows PowerShell 5.1 reads BOM-less scripts as the ANSI code
# page. Exit code 0 = all checks passed.
param(
    [Parameter(Mandatory = $true)][string]$Jira,
    [Parameter(Mandatory = $true)][string]$Work
)

$ErrorActionPreference = 'Continue'   # native stderr must reach the pipeline
$Jira = (Resolve-Path $Jira).Path
New-Item -ItemType Directory -Force -Path $Work | Out-Null
$Work = (Resolve-Path $Work).Path
$script:pass = 0; $script:fail = 0; $script:skip = 0
$log = New-Object System.Collections.Generic.List[string]

function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    if ($Ok) { $script:pass++; $log.Add("PASS  $Name") }
    else { $script:fail++; $log.Add("FAIL  $Name  --  $Detail") }
}
function Skip([string]$Name, [string]$Why) { $script:skip++; $log.Add("SKIP  $Name  --  $Why") }
function Run([string[]]$JiraArgs, [switch]$MergeStderr) {
    if ($MergeStderr) { & $Jira @JiraArgs 2>&1 | ForEach-Object { "$_" } }
    else { & $Jira @JiraArgs }
}
function StrictUtf8([byte[]]$Bytes) {
    try { return [Text.Encoding]::GetEncoding('utf-8', [Text.EncoderFallback]::ExceptionFallback, [Text.DecoderFallback]::ExceptionFallback).GetString($Bytes) }
    catch { return $null }
}
function ConsoleCanEncode([string]$Text) {
    $enc = [Console]::OutputEncoding
    return $enc.GetString($enc.GetBytes($Text)) -ceq $Text
}

# A config file with dummy credentials: `auth status` is offline.
$env:JIRA_CONFIG_FILE = Join-Path $Work 'config.toml'
[IO.File]::WriteAllText($env:JIRA_CONFIG_FILE, "url = `"http://127.0.0.1:9`"`nauth_type = `"basic`"`nuser = `"u`"`ntoken = `"t`"`n")

$token = '(' + [string][char]0x5DF2 + [char]0x8BBE + [char]0x7F6E + [char]0xFF0C + [char]0x5DF2 + [char]0x8131 + [char]0x654F + ')'
$summary = [string][char]0x6D4B + [char]0x8BD5 + ' ' + [char]::ConvertFromUtf32(0x1F680) + ' ' + [char]::ConvertFromUtf32(0x2000B)

# ---- reference: the UTF-8 console a user could set up by hand ---------------
$original = [Console]::OutputEncoding
$startCodePage = $original.CodePage
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$utf8CodePage = [Console]::OutputEncoding.CodePage
$truthToken = (Run @('auth', 'status') | ConvertFrom-Json).token
$truthError = (Run @('update', 'X-1') -MergeStderr) -join "`n"

# ---- the console under test --------------------------------------------------
# A stock PowerShell decodes with the console output code page (936 on zh-CN).
# Where the console is already UTF-8 (CI runners), switch it to 936 so that
# the conversion under test is really exercised.
$codePage = if ($startCodePage -eq 65001) { 936 } else { $startCodePage }
[Console]::OutputEncoding = [Text.Encoding]::GetEncoding($codePage)
$testEncoding = [Console]::OutputEncoding
$log.Add("PowerShell $($PSVersionTable.PSVersion); console code page under test $([Console]::OutputEncoding.CodePage), wanted $codePage (script started at $startCodePage)")
$log.Add("reference: token=$truthToken; error=$truthError")

Check 'reference run really used a UTF-8 console' ($utf8CodePage -eq 65001) "code page $utf8CodePage"
Check 'the console under test is the intended non-UTF-8 one' ([Console]::OutputEncoding.CodePage -eq $codePage -and $codePage -ne 65001) "code page $([Console]::OutputEncoding.CodePage), wanted $codePage"
Check 'reference token is the expected Chinese text' ($truthToken -ceq $token) "got '$truthToken'"

# ---- nothing configured: what a user sees by default -------------------------
$obj = Run @('auth', 'status') | ConvertFrom-Json
Check 'plain run shows Chinese correctly' ($obj.token -ceq $truthToken) "got '$($obj.token)'"

$dry = Run @('create', 'X', 'Task', $summary, '--dry-run') | ConvertFrom-Json
Check 'JSON stays lossless for characters the code page lacks' ($dry.body.fields.summary -ceq $summary) "got '$($dry.body.fields.summary)'"

if (ConsoleCanEncode $truthError) {
    $err = (Run @('update', 'X-1') -MergeStderr) -join "`n"
    Check 'Chinese diagnostics via 2>&1 survive' ($err -ceq $truthError) "got '$err'"
}
else {
    Skip 'Chinese diagnostics via 2>&1 survive' "code page $codePage cannot represent them; diagnostics are lossy there by design"
}

# What PowerShell reads is whatever its own encoding decodes correctly.
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Jira; $psi.Arguments = 'auth status'; $psi.UseShellExecute = $false; $psi.RedirectStandardOutput = $true
$p = [System.Diagnostics.Process]::Start($psi)
$bytes = New-Object IO.MemoryStream
$p.StandardOutput.BaseStream.CopyTo($bytes); $p.WaitForExit()
$wire = [Console]::OutputEncoding.GetString($bytes.ToArray()) | ConvertFrom-Json
Check 'the bytes on the pipe decode with the console code page' ($wire.token -ceq $truthToken) "decoded as '$($wire.token)'"

$f = Join-Path $Work 'redirect.txt'
Run @('auth', 'status') > $f
Check 'PowerShell > redirect keeps the text' ((Get-Content $f -Raw).Contains($truthToken)) 'token text not found'

$f = Join-Path $Work 'to-file.txt'
Start-Process -FilePath $Jira -ArgumentList 'auth', 'status' -RedirectStandardOutput $f -NoNewWindow -Wait
$text = StrictUtf8 ([IO.File]::ReadAllBytes($f))
Check 'output redirected to a file stays UTF-8' ($null -ne $text -and $text.Contains($truthToken)) 'file is not UTF-8 with the expected text'

$f = Join-Path $Work 'via-cmd.txt'
cmd /c "`"$Jira`" auth status > `"$f`""
$text = StrictUtf8 ([IO.File]::ReadAllBytes($f))
Check 'a cmd.exe parent gets UTF-8' ($null -ne $text -and $text.Contains($truthToken)) 'file is not UTF-8 with the expected text'

$f = Join-Path $Work 'via-bash.txt'
if (Get-Command bash -ErrorAction SilentlyContinue) {
    & bash -c "'$($Jira.Replace('\', '/'))' auth status | cat > '$($f.Replace('\', '/'))'"
    if ($LASTEXITCODE -eq 0 -and (Test-Path $f)) {
        $text = StrictUtf8 ([IO.File]::ReadAllBytes($f))
        Check 'a bash parent gets UTF-8 (even when PowerShell started the bash)' ($null -ne $text -and $text.Contains($truthToken)) 'file is not UTF-8 with the expected text'
    }
    else {
        Skip 'a bash parent gets UTF-8 (even when PowerShell started the bash)' "bash exited $LASTEXITCODE"
    }
}
else {
    Skip 'a bash parent gets UTF-8 (even when PowerShell started the bash)' 'bash not available'
}

# MCP must stay UTF-8 even though PowerShell is the parent process here.
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Jira; $psi.Arguments = 'mcp'; $psi.UseShellExecute = $false
$psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true
$psi.EnvironmentVariables['JIRA_CONFIG_FILE'] = Join-Path $Work 'does-not-exist.toml'
$p = [System.Diagnostics.Process]::Start($psi)
$p.StandardInput.Write('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}' + "`n" +
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' + "`n" +
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"test_auth","arguments":{}}}' + "`n")
$p.StandardInput.Close()
$bytes = New-Object IO.MemoryStream
$p.StandardOutput.BaseStream.CopyTo($bytes); $p.WaitForExit()
$mcp = StrictUtf8 ($bytes.ToArray())
Check 'the MCP transport stays raw UTF-8 under a PowerShell parent' ($null -ne $mcp -and $mcp.Contains([string][char]0x672A + [char]0x627E + [char]0x5230)) 'MCP output is not UTF-8 with the expected message'

# ---- the knobs ---------------------------------------------------------------
$env:JIRA_OUTPUT_ENCODING = 'utf-8'
$forced = (Run @('auth', 'status') | ConvertFrom-Json).token
Remove-Item Env:JIRA_OUTPUT_ENCODING
Check 'JIRA_OUTPUT_ENCODING=utf-8 turns the conversion off' ($forced -cne $truthToken) "still converted: '$forced'"

[Console]::OutputEncoding = [Text.Encoding]::UTF8
$utf8Console = (Run @('auth', 'status') | ConvertFrom-Json).token
[Console]::OutputEncoding = $testEncoding
Check 'a UTF-8 console keeps working' ($utf8Console -ceq $truthToken) "got '$utf8Console'"

[Console]::OutputEncoding = $original
Check 'console code page restored' ([Console]::OutputEncoding.CodePage -eq $startCodePage) "now $([Console]::OutputEncoding.CodePage), started at $startCodePage"

$log | ForEach-Object { Write-Output $_ }
Write-Output ("{0} passed, {1} failed, {2} skipped" -f $script:pass, $script:fail, $script:skip)
if ($script:fail) { exit 1 }
