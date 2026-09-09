<#
.SYNOPSIS
    Lints and tests the workspace backup script.

.DESCRIPTION
    One entry point, so the pipeline and a person at a prompt run exactly
    the same thing. Fails on the first stage that fails rather than
    reporting all of them: a script that does not lint is not worth
    testing.

.EXAMPLE
    ./build.ps1 -Task Test
#>
[CmdletBinding()]
param(
    [ValidateSet('Lint', 'Test', 'All')]
    [string] $Task = 'All'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Lint {
    Write-Verbose 'Running PSScriptAnalyzer...'
    $findings = Invoke-ScriptAnalyzer -Path $PSScriptRoot -Recurse `
        -Settings (Join-Path $PSScriptRoot 'PSScriptAnalyzerSettings.psd1')

    if ($findings) {
        $findings | Format-Table -AutoSize
        throw "$($findings.Count) analyzer finding(s)"
    }
}

function Invoke-Test {
    Write-Verbose 'Running Pester...'
    $configuration = New-PesterConfiguration
    $configuration.Run.Path = Join-Path $PSScriptRoot 'Tests'
    $configuration.Output.Verbosity = 'Detailed'
    $configuration.Run.Exit = $true

    Invoke-Pester -Configuration $configuration
}

switch ($Task) {
    'Lint' { Invoke-Lint }
    'Test' { Invoke-Test }
    'All' { Invoke-Lint; Invoke-Test }
}
