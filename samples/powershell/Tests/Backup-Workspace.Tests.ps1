#Requires -Module Pester

Describe 'Backup-Workspace' {

    BeforeAll {
        $script:ScriptPath = Join-Path $PSScriptRoot '..' 'Backup-Workspace.ps1'
    }

    It 'is there to be run' {
        Test-Path $script:ScriptPath | Should -BeTrue
    }

    It 'parses without executing' {
        # Parsing separately from running is the cheapest test there is,
        # and it catches the mistake that stops every other test from
        # running at all.
        $errors = $null
        [System.Management.Automation.Language.Parser]::ParseFile(
            $script:ScriptPath, [ref] $null, [ref] $errors) | Out-Null

        $errors | Should -BeNullOrEmpty
    }

    It 'supports -WhatIf, so a dry run is possible' {
        $ast = [System.Management.Automation.Language.Parser]::ParseFile(
            $script:ScriptPath, [ref] $null, [ref] $null)

        $ast.Extent.Text | Should -Match 'SupportsShouldProcess'
    }

    It 'declares comment-based help' {
        $ast = [System.Management.Automation.Language.Parser]::ParseFile(
            $script:ScriptPath, [ref] $null, [ref] $null)

        $ast.GetHelpContent().Synopsis | Should -Not -BeNullOrEmpty
    }
}
