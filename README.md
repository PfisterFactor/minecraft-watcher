# Minecraft Status Watcher

## What is this?
A little command line program I whipped up to monitor my EC2 Minecraft Server and allow players to automatically start it when they wanted to play, then shut it down after `inactivity_timer` minutes.

## Features
This program will:
- Check the player count on the Minecraft server every minute, and - if there are no players for `inactivity_timer` minutes - shut down the EC2 instance
- Spoof the Minecraft server protocol to allow users to check the status of the server from the Multiplayer screen and - if allowed to - start the EC2 instance

## Building
Using Rust `1.77.2` nightly:
```
git clone https://github.com/PfisterFactor/minecraft-watcher.git
cd minecraft-watcher
cargo +nightly build
```
## Requirements


This program assumes:
- Your Minecraft server is being hosted on an EC2 instance
    - Spot instances are supported
- Your Minecraft server is configured to automatically startup when the instance starts
- Minecraft server v1.15.2 or greater

## Example setup
In my configuration, I have a two EC2 instances, `minecraft-server` and `minecraft-watcher`.

The `minecraft-watcher` EC2 instance is a low cost, spot, nano instance that runs this program.

The `minecraft-server` EC2 instance is a higher tier instance that runs the actual minecraft server.

Additionally, I have a Route53 hosted zone with two Primary/Secondary failover records (with a configured health check for the primary):
- Primary: `minecraft-server`
- Secondary: `minecraft-watcher`

This allows me to have a DNS record that will direct players to the server if it is online, but to the watcher if it is offline - meaning players only need one minecraft server entry in their Multiplayer screen.

## Usage

```
Usage: mc-server-init [OPTIONS] --ec2-instance <EC2_INSTANCE>

Options:
      --ec2-instance <EC2_INSTANCE>
          EC2 Instance ID to monitor
      --watcher-port <WATCHER_PORT>
          TCP Port to have the watcher listen on [default: 25565]
      --server-port <SERVER_PORT>
          TCP Port the remote minecraft server is running on [default: 25565]
      --inactivity-timer <INACTIVITY_TIMER>
          Minutes to wait before considering the server as inactive and shutting it down [default: 20]
      --usernames-allowed-to-start-server <USERNAMES_ALLOWED_TO_START_SERVER>
          List of usernames allowed to start the server seperated by commas, or '*' for everyone allowed
  -h, --help
          Print help
```
