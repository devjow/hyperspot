# UPSTREAM_REQS — Notification Service


<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Requesting Modules](#12-requesting-modules)
- [2. Requirements](#2-requirements)
  - [2.1 Todo App](#21-todo-app)
- [3. Priorities](#3-priorities)
- [4. Traceability](#4-traceability)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

A centralized notification service is needed to deliver reminders and alerts to users across multiple channels (push, email, in-app). The Todo App currently references a Notification Service system actor (`cpt-examples-todo-app-actor-notification-service`) but no module exists to fulfill this role.

### 1.2 Requesting Modules

| Module | Why it needs this module |
|--------|-------------------------|
| todo-app | Needs to send task reminders for upcoming and overdue tasks to keep users on track |

## 2. Requirements

### 2.1 Todo App

#### Send Task Reminder

- [ ] `p1` - **ID**: `cpt-examples-notification-service-upreq-send-task-reminder`

The future module **MUST** accept a reminder request containing a user ID, message text, and delivery time, and deliver the notification at the specified time.

- **Rationale**: Todo App tracks task due dates and needs a reliable way to notify users about upcoming deadlines without implementing delivery logic itself.
- **Source**: `modules/todo-app`

#### Support Multiple Channels

- [ ] `p2` - **ID**: `cpt-examples-notification-service-upreq-multi-channel`

The future module **MUST** support at least two delivery channels: in-app notification and email.

- **Rationale**: Users may not be actively using the app when a task is due; email ensures the reminder reaches them regardless.
- **Source**: `modules/todo-app`

#### Cancel Scheduled Reminder

- [ ] `p2` - **ID**: `cpt-examples-notification-service-upreq-cancel-reminder`

The future module **MUST** allow cancellation of a previously scheduled reminder by its identifier.

- **Rationale**: When a user completes or deletes a task before its due date, the associated reminder must be cancelled to avoid confusing notifications.
- **Source**: `modules/todo-app`

#### Delivery Confirmation

- [ ] `p3` - **ID**: `cpt-examples-notification-service-upreq-delivery-confirmation`

The future module **MUST** provide a way to query whether a specific notification was successfully delivered.

- **Rationale**: Todo App may display a "reminder sent" indicator in the UI; it needs to know if delivery actually succeeded.
- **Source**: `modules/todo-app`

## 3. Priorities

| Priority | Requirements |
|----------|-------------|
| p1 (critical) | `cpt-examples-notification-service-upreq-send-task-reminder` |
| p2 (important) | `cpt-examples-notification-service-upreq-multi-channel`, `cpt-examples-notification-service-upreq-cancel-reminder` |
| p3 (nice-to-have) | `cpt-examples-notification-service-upreq-delivery-confirmation` |

## 4. Traceability

- **PRD** (when created): [PRD.md](./PRD.md)
- **Design** (when created): [DESIGN.md](./DESIGN.md)
