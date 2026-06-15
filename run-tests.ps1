# Run ViCell integration tests.
#
# Integration tests are host binaries (they spawn QEMU as a subprocess).
# The .cargo/config.toml at the repo root sets target=riscv64gc, so we must
# cd into tests/integration/ to let its own .cargo/config.toml override the target.
#
# Usage:
#   ./run-tests.ps1                     # run all integration tests
#   ./run-tests.ps1 boot                # run only the "boot" test suite
#   ./run-tests.ps1 boot input_bare_cell # run specific test function

param(
    [string]$Suite = "",
    [string]$TestName = ""
)

$repo = Get-Location

Push-Location "$repo\tests\integration"
try {
    $args_list = @("test")
    if ($Suite)     { $args_list += "--test"; $args_list += $Suite }
    if ($TestName)  { $args_list += $TestName }
    cargo @args_list
} finally {
    Pop-Location
}
