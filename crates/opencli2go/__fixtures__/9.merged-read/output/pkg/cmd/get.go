package cmd

import (
	"context"
	"net/url"

	"example.com/acme/internal/runtime"
	"github.com/urfave/cli/v3"
)

func NewGetCommand() *cli.Command {
	return &cli.Command{
		Name: "get",
		Commands: []*cli.Command{
			&cli.Command{
				Name: "sdk",
				Aliases: []string{
					"sdks",
				},
				Usage: "List SDKs, or retrieve one by id.",
				Flags: []cli.Flag{
					&cli.StringFlag{
						Name: "limit",
					},
				},
				Action: handleGetSdk,
				Commands: []*cli.Command{
					&cli.Command{
						Name: "targets",
						Action: handleGetSdkTargets,
					},
				},
			},
		},
	}
}

func handleGetSdk(ctx context.Context, cmd *cli.Command) error {
	id := cmd.Args().Get(0)
	useAlt := id != ""
	method, path := "GET", "/sdks"
	if useAlt {
		method, path = "GET", "/sdks/" + url.PathEscape(id)
	}
	query := url.Values{}
	if !useAlt && cmd.IsSet("limit") {
		query.Set("limit", cmd.String("limit"))
	}
	req := runtime.Request{
		Method: method,
		Path: path,
		Query: query,
	}
	return runtime.Do(ctx, req)
}

func handleGetSdkTargets(ctx context.Context, cmd *cli.Command) error {
	sdkID := cmd.Args().Get(0)
	path := "/sdks/" + url.PathEscape(sdkID) + "/targets"
	req := runtime.Request{
		Method: "GET",
		Path: path,
	}
	return runtime.Do(ctx, req)
}
