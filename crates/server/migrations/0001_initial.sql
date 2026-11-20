-- One person's gym data. Weights are grams, times are UTC.

create table settings (
    key text primary key,
    value jsonb not null
);

-- Sign-in
create table passkeys (
    id uuid primary key,
    name text not null,
    cred jsonb not null,
    created_at timestamptz not null default now(),
    last_used_at timestamptz
);

create table auth_sessions (
    token_hash text primary key,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null,
    last_seen_at timestamptz not null default now()
);

create table ceremonies (
    id uuid primary key,
    kind text not null,
    state jsonb not null,
    expires_at timestamptz not null
);

-- Lifts and plans
create table lifts (
    id uuid primary key,
    name text not null,
    norm_name text not null,
    tags jsonb,
    tag_source text,
    sets int not null,
    reps int not null,
    weight_g bigint not null,
    rest_s int,
    cues jsonb not null default '[]',
    archived boolean not null default false,
    position int not null default 0,
    created_at timestamptz not null default now(),
    created_by text not null
);
create unique index lifts_one_active_name on lifts (norm_name) where not archived;

create table clips (
    id uuid primary key,
    lift_id uuid not null references lifts (id) on delete cascade,
    url text not null,
    provider text not null,
    title text not null,
    added_by text not null,
    position int not null default 0,
    created_at timestamptz not null default now()
);
create index clips_by_lift on clips (lift_id, position);

create table plan_versions (
    id uuid primary key,
    previous_id uuid references plan_versions (id),
    created_at timestamptz not null default now(),
    created_by text not null,
    data jsonb not null
);

-- Training
create table workout_sessions (
    id uuid primary key,
    started_at timestamptz not null,
    closed_at timestamptz,
    close_reason text,
    day_name text not null,
    day_index int,
    last_set_at timestamptz,
    workout jsonb,
    created_at timestamptz not null default now()
);
-- At most one session is open at a time.
create unique index one_open_session on workout_sessions ((true)) where closed_at is null;

create table sets (
    id uuid primary key,
    session_id uuid not null references workout_sessions (id) on delete cascade,
    lift_id uuid not null references lifts (id),
    weight_g bigint not null,
    reps int not null,
    unit text not null,
    logged_at timestamptz not null,
    created_at timestamptz not null default now()
);
create index sets_by_session on sets (session_id, logged_at);
create index sets_by_lift on sets (lift_id, logged_at);

-- Apple Health (via iPhone Shortcuts), Lifesum day totals ride along
create table health_days (
    date date primary key,
    energy_burned_kcal double precision,
    energy_eaten_kcal double precision,
    protein_g double precision,
    carbs_g double precision,
    fat_g double precision,
    updated_at timestamptz not null default now()
);

create table health_workouts (
    id text primary key,
    kind text not null,
    start_at timestamptz not null,
    end_at timestamptz not null,
    energy_kcal double precision,
    avg_hr double precision,
    max_hr double precision,
    distance_km double precision,
    session_id uuid references workout_sessions (id) on delete set null,
    received_at timestamptz not null default now()
);

create table close_signals (
    id bigserial primary key,
    received_at timestamptz not null default now(),
    kind text not null,
    detail jsonb not null,
    outcome text not null
);

-- Agents
create table agents (
    id uuid primary key,
    name text not null,
    key_hash text not null unique,
    key_prefix text not null,
    ask_first boolean not null default false,
    created_at timestamptz not null default now(),
    last_used_at timestamptz,
    revoked_at timestamptz
);

create table activity (
    id uuid primary key,
    agent_id uuid references agents (id),
    agent_name text not null,
    at timestamptz not null default now(),
    action text not null,
    summary text not null,
    detail jsonb not null default '[]',
    status text not null,
    op jsonb not null,
    undo jsonb,
    resolved_at timestamptz
);
create index activity_by_time on activity (at desc);

-- Strava and Spotify: tokens are encrypted with a key that lives only in the environment
create table provider_tokens (
    provider text primary key,
    ciphertext bytea not null,
    nonce bytea not null,
    account text,
    expires_at timestamptz,
    connected_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table oauth_states (
    state text primary key,
    provider text not null,
    expires_at timestamptz not null
);

-- Strava's terms cap storage at seven days; rows older than that are deleted.
create table strava_activities (
    id bigint primary key,
    data jsonb not null,
    start_at timestamptz not null,
    fetched_at timestamptz not null default now()
);
