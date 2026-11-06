# Security policy

## Reporting a vulnerability

Please report it privately, not in a public issue or discussion.

Use GitHub's private vulnerability reporting: open the repository's **Security** tab and choose **Report a
vulnerability**. Include what you found, how to reproduce it (a request, a configuration, a version), and what you
think the impact is. If you are not sure whether something is a vulnerability, report it anyway.

What to expect, from a small project run by one maintainer: an acknowledgement within 7 days, an assessment and a
plan within 30, and a fix released as soon as it is ready. I will credit you in the release notes unless you prefer
not to be named. I ask for up to 90 days to fix and release before details are made public, and sooner if a fix ships
earlier; if a vulnerability is already being exploited, tell me and we will move faster.

## Supported versions

Only the latest release receives security fixes. Upgrading is a drop-in replacement of the binary or the image; read
the release notes for anything that changes.

## What is in scope

Things that let someone do what they should not be able to do, for example: using the gateway without a valid key or
beyond a key's route and budget limits; reaching the admin API without an admin credential or with too few scopes;
reading a provider key, a client key or an admin credential; making the gateway send a key to a host other than the
provider it belongs to; reading or altering another party's data; code execution; and crashing or exhausting the
gateway with a small request.

## What is not

A person who already has the configuration file, the environment or the host; a deployment that exposes the gateway
or the admin API over plain HTTP on an untrusted network (put TLS in front of it); the behaviour of a provider or a
model; and findings that need a misconfiguration the documentation warns against. Reports about these are still
welcome as ordinary issues.

## Research

Testing your own deployment is always fine. Please do not test against other people's deployments or degrade the
service for anyone, and do not access or keep data beyond what is needed to show the problem. Acting in good faith
within this policy, I will not pursue action against you. There is no bounty.

How the project thinks about its threats, and which ones have no answer yet, is in the
[threat model](https://github.com/gschaetz/humpyard/blob/main/docs/threat-model.md).
