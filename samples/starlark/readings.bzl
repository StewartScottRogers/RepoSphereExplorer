"""The rule that generates a fixture of readings for the tests."""

load("@bazel_skylib//lib:paths.bzl", "paths")

def _readings_fixture_impl(ctx):
    out = ctx.actions.declare_file(paths.replace_extension(ctx.label.name, ".csv"))
    ctx.actions.run(
        outputs = [out],
        executable = ctx.executable._generator,
        arguments = [
            "--rows",
            str(ctx.attr.rows),
            "--seed",
            str(ctx.attr.seed),
            "--out",
            out.path,
        ],
        mnemonic = "GenerateReadings",
        progress_message = "Generating %d reading(s) for %s" % (ctx.attr.rows, ctx.label),
    )
    return [DefaultInfo(files = depset([out]))]

readings_fixture = rule(
    implementation = _readings_fixture_impl,
    doc = "Writes a CSV of plausible readings for a test to read.",
    attrs = {
        "rows": attr.int(
            default = 100,
            doc = "How many readings to write.",
        ),
        "seed": attr.int(
            default = 1,
            doc = "The seed, so the same rows come out every time.",
        ),
        "_generator": attr.label(
            default = Label("//tools:generate_readings"),
            executable = True,
            cfg = "exec",
        ),
    },
)
