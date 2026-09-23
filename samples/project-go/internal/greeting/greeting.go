// Package greeting builds the message the greeter command prints for a
// name. It is kept internal because the phrasing is an implementation
// detail of this one command, not something another module should import.
package greeting

import "strings"

// For returns the greeting for name, treating an empty name as "there"
// rather than printing an empty salutation.
func For(name string) string {
	trimmed := strings.TrimSpace(name)
	if trimmed == "" {
		trimmed = "there"
	}
	return "Hello, " + trimmed + "!"
}
