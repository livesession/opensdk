package runtime

import (
	"net/http"
	"os"
)

const defaultBaseURL = "https://api.acme.test"

// BaseURL returns the API base URL, overridable via ACME_BASE_URL.
func BaseURL() string {
	if v := os.Getenv("ACME_BASE_URL"); v != "" {
		return v
	}
	return defaultBaseURL
}

// applyAuth attaches credentials read from the environment to the request.
func applyAuth(req *http.Request) {}
