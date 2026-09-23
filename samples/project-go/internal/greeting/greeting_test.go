package greeting

import "testing"

func TestForTrimsAndDefaultsAnEmptyName(t *testing.T) {
	cases := map[string]string{
		"Ada":   "Hello, Ada!",
		"  Bo ": "Hello, Bo!",
		"":      "Hello, there!",
	}

	for input, want := range cases {
		if got := For(input); got != want {
			t.Errorf("For(%q) = %q, want %q", input, got, want)
		}
	}
}
