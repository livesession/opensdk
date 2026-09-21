package main

import (
	"context"
	"log"
	"os"

	"example.com/acme/pkg/cmd"
	"github.com/urfave/cli/v3"
)

func main() {
	app := &cli.Command{
		Name: "acme",
		Version: "1.0.0",
		Commands: []*cli.Command{
			cmd.NewGetCommand(),
		},
	}
	if err := app.Run(context.Background(), os.Args); err != nil {
		log.Fatal(err)
	}
}
