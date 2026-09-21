package cmd

import (
	"context"
	"net/url"

	"example.com/api/internal/runtime"
	"github.com/urfave/cli/v3"
)

func NewGetCommand() *cli.Command {
	return &cli.Command{
		Name: "get",
		Commands: []*cli.Command{
			&cli.Command{
				Name: "sdks",
				Aliases: []string{
					"sdk",
				},
				Usage: "List SDKs, or retrieve one by id",
				Flags: []cli.Flag{
					&cli.StringFlag{
						Name: "limit",
						Usage: "Page size",
					},
				},
				Action: handleGetSdks,
				Commands: []*cli.Command{
					&cli.Command{
						Name: "targets",
						Usage: "List an SDK's targets",
						Action: handleGetSdksTargets,
					},
				},
			},
		},
	}
}

func handleGetSdks(ctx context.Context, cmd *cli.Command) error {
	path := "/sdks"
	query := url.Values{}
	if cmd.IsSet("limit") {
		query.Set("limit", cmd.String("limit"))
	}
	req := runtime.Request{
		Method: "GET",
		Path: path,
		Query: query,
	}
	return runtime.Do(ctx, req)
}

func handleGetSdksTargets(ctx context.Context, cmd *cli.Command) error {
	sdkID := cmd.Args().Get(0)
	path := "/sdks/" + url.PathEscape(sdkID) + "/targets"
	req := runtime.Request{
		Method: "GET",
		Path: path,
	}
	return runtime.Do(ctx, req)
}
