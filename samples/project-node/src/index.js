import { Command } from "commander";
import chalk from "chalk";

/**
 * Builds the CLI without running it, so a test can drive it without
 * touching `process.argv` or actually printing anything.
 */
export function buildCli() {
  const program = new Command();
  program
    .name("factory-floor")
    .description("Reports on the repositories a floor watches over.")
    .argument("<count>", "how many repositories were checked")
    .action((count) => {
      console.log(chalk.green(`Checked ${count} repositories.`));
    });
  return program;
}

export class ReportError extends Error {
  constructor(message, exitCode) {
    super(message);
    this.name = "ReportError";
    this.exitCode = exitCode;
  }
}
