package main

import (
	"context"
	"log"
	"os"

	"example.com/api/pkg/cmd"
	"github.com/urfave/cli/v3"
)

func main() {
	app := &cli.Command{
		Name: "api",
		Usage: "A command that is BOTH runnable and a parent of subcommands.",
		Version: "1.0.0",
		Commands: []*cli.Command{
			cmd.NewGetCommand(),
		},
	}
	if err := app.Run(context.Background(), os.Args); err != nil {
		log.Fatal(err)
	}
}
