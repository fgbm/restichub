-- The runtime role removes the expired and the used setup tokens (issue #18).
--
-- Migration 0010 gave the runtime role SELECT, INSERT and UPDATE on `setup_tokens`. The control
-- plane maintenance task also removes the expired and the used tokens, so the role needs DELETE.

GRANT DELETE ON setup_tokens TO restichub_app;
