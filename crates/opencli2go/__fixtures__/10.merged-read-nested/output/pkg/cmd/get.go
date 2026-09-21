package cmd

import (
	"context"
	"fmt"
	"net/url"

	"example.com/acme/internal/runtime"
	"github.com/urfave/cli/v3"
)

func NewGetCommand() *cli.Command {
	return &cli.Command{
		Name: "get",
		Commands: []*cli.Command{
			&cli.Command{
				Name: "org",
				Commands: []*cli.Command{
					&cli.Command{
						Name: "sdk",
						Aliases: []string{
							"sdks",
						},
						Flags: []cli.Flag{
							&cli.IntFlag{
								Name: "limit",
							},
							&cli.StringFlag{
								Name: "include",
							},
						},
						Action: handleGetOrgSdk,
					},
				},
			},
		},
	}
}

func handleGetOrgSdk(ctx context.Context, cmd *cli.Command) error {
	orgID := cmd.Args().Get(0)
	sdkID := cmd.Args().Get(1)
	useAlt := sdkID != ""
	method, path := "GET", "/orgs/" + url.PathEscape(orgID) + "/sdks"
	if useAlt {
		method, path = "GET", "/orgs/" + url.PathEscape(orgID) + "/sdks/" + url.PathEscape(sdkID)
	}
	query := url.Values{}
	if !useAlt && cmd.IsSet("limit") {
		query.Set("limit", fmt.Sprint(cmd.Int("limit")))
	}
	if useAlt && cmd.IsSet("include") {
		query.Set("include", cmd.String("include"))
	}
	req := runtime.Request{
		Method: method,
		Path: path,
		Query: query,
	}
	return runtime.Do(ctx, req)
}
