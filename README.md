rbqm-link-checker
=================

rbqm-link-checker is a tool to check the status of the links in the RBQM dashboard. It uses the cookie value to authenticate the user and then
incrementally crawls the entire site for all links to check their validity.
For any valid links on a page it follows any that match the domain
to ultimately check every link a user would have access to across the site.


To run the development version of the tool via cargo:

```
cargo run -- --domain rbqm-dashboard-dev.gilead.com --cookie-value=PFa...FULLCOOKIE...IkZvr0=
```

You can also use `.env` file to store the domain and cookie values.
Check the `.env.sample` file for the format.

We'll build a binary once this gets a couple reps in and distribute it that way in the future.
