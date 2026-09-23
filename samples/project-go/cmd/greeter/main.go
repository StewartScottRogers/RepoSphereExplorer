// Command greeter prints a greeting for each name given on the command
// line, tagging the run with a fresh identifier.
package main

import (
	"fmt"
	"os"

	"github.com/google/uuid"
	"github.com/spf13/cobra"

	"github.com/example/greeter/internal/greeting"
)

func main() {
	root := &cobra.Command{
		Use:   "greeter [names...]",
		Short: "Greet the names given on the command line",
		RunE: func(_ *cobra.Command, args []string) error {
			runID := uuid.New()
			for _, name := range args {
				fmt.Printf("[%s] %s\n", runID, greeting.For(name))
			}
			return nil
		},
	}

	if err := root.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
