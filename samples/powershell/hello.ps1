<#
.SYNOPSIS
  Greets someone.
#>
param(
    [string]$Name = "World"
)

Write-Host "Hello, $Name"
