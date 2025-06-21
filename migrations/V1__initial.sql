CREATE TABLE `bot_group_member` (
    `id` INTEGER NOT NULL PRIMARY KEY,
    `created_at` TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    `qq_uid` TEXT NOT NULL,
    `qq_uin` INTEGER NOT NULL,
    `nickname` TEXT NOT NULL,
    `group_nickname` TEXT NOT NULL,
    `sort_key` INTEGER NOT NULL DEFAULT 3001
);

CREATE UNIQUE INDEX idx_bot_group_member_qq_uid ON bot_group_member (qq_uid);

CREATE TABLE bot_daka (
    `id` INTEGER NOT NULL PRIMARY KEY,
    `user_id` INTEGER NOT NULL REFERENCES `bot_group_member`(`id`) ON DELETE NO ACTION,
    `created_at` TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
    `note` TEXT NOT NULL DEFAULT ''
);

CREATE INDEX idx_bot_daka_created_at ON bot_daka (created_at);
