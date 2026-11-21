-- Strava is gone: its API needs a paid subscription. Drop what it stored.
drop table if exists strava_activities;
delete from provider_tokens where provider = 'strava';
