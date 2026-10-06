# Инвентарь регрессий I01–I03

Дата: 2026-10-06. Задача `I03.T05.a`. Каждая проверка имеет ID, уровень доказательства и статус.
Локальный статус `PASS locally` относится к unit/fixture (`cargo test --offline --locked`). Он не повышает runtime-приёмку: установка, живые подписки и сеть хоста остаются `NOT_RUN` до трека X.

| ID | Файл | Уровень | Статус | Примечание |
|---|---|---|---|---|
| `R001` `helper_framing_preserves_events_coalesced_with_reply` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R002` `helper_framing_reports_malformed_event_json` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R003` `helper_client_rejects_old_server_before_mutation` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R004` `new_helper_rejects_legacy_mutation_without_protocol_version` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R005` `audit_term_bounds_long_lines` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R006` `audit_utf8_preserves_valid_tail_after_invalid_byte` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R007` `audit_closed_attach_releases_helper_thread_and_fd` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R008` `helper_fixture_runs_fake_child_and_reaps_helper` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R009` `helper_runner_launch_failures_finish_and_clean_up` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R010` `helper_runner_drains_descendant_and_switches_without_stale_output` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R011` `helper_runner_cancel_stops_continuous_output` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R012` `helper_idle_exit_after_last_subscription` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R013` `helper_secret_answer_is_not_echoed_or_replayed` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R014` `config_save_merges_stale_fields_and_mirror_changes` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R015` `two_helper_processes_preserve_distinct_config_fields` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R016` `config_rejects_dns_collision_and_reports_pending_then_apply_failure` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R017` `waiting_config_file_lock_does_not_block_status_or_cancel` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R018` `concurrent_cli_and_helper_write_complete_latest_vpn_yaml` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R019` `explicit_user_context_keeps_proxy_uids_and_environment_separate` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R020` `cli_and_inline_configset_use_same_peer_proxy_context_and_report_errors` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R021` `helper_subprocess_receives_peer_identity_without_ambient_daemon_user` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R022` `helper_overload_has_resource_plateau_and_returns_fds_threads_children` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R023` `concurrent_cli_language_and_helper_field_preserve_separate_state_files` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R024` `helper_restart_does_not_fabricate_success_for_untracked_operation` | `tests/audit_contracts.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R025` `two_namespace_harness_reaches_blocks_and_reopens` | `tests/audit_h11_netharness.rs` | L2 | PASS locally | smoke veth/nft; worker-сценарий — отдельная команда |
| `R026` `i01_fresh_and_cm_only_pass_without_any_write` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R027` `i01_old_only_and_all_old_new_conflicts_preserve_full_contents` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R028` `i01_foreign_hooks_cron_desktop_cli_and_policy_are_refused_and_preserved` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R029` `i01_units_dropins_and_enabled_links_refuse_before_ownership_changes` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R030` `i01_dangling_final_and_ancestor_symlinks_never_follow_outside_root` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R031` `i01_non_directory_ancestor_and_invalid_root_are_fail_closed` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R032` `i01_metadata_permission_error_and_symlink_root_are_refused` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R033` `i01_helper_socket_is_refused_without_disconnecting_or_unlinking` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R034` `i01_cm_and_legacy_test_environment_cannot_bypass_production_preflight` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R035` `c17a_clean_root_and_unprivileged_readers_pass_without_writes` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R036` `c17a_privileged_startup_refuses_every_dispatch_before_any_write` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R037` `c17a_var_lib_only_legacy_and_unprivileged_install_refuse_without_writes` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R038` `c17a_cm_and_upd_environment_overrides_do_not_bypass_root_guard` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R039` `c17a_unsafe_legacy_ancestor_is_refused_for_root_startup` | `tests/audit_i01_install_guard.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R040` `manual_plan_prevalidates_nested_data_without_writing_journal` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R041` `shared_plan_validator_success_is_read_only` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R042` `success_copies_bytes_owner_mode_and_service_state` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R043` `rejects_foreign_ancestor_and_busy_flock_without_mutation` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R044` `rejects_final_symlink_target_and_overlapping_plan_before_journaling` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R045` `manual_policy_applies_pinned_binary_layout_with_exact_references_and_replacements` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R046` `observer_failures_roll_back_each_durable_boundary` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R047` `crash_worker` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R048` `killed_process_recovers_from_persistent_journal_in_new_process` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R049` `recovery_refuses_tampered_backup_and_foreign_target_edits` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R050` `incomplete_rollback_can_be_retried_without_losing_backup` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R051` `recovery_cleans_only_verified_partial_staging_prefixes` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R052` `recovery_preserves_unrecognized_journal_staging_contents` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R053` `committed_plan_repeat_verifies_without_recopying_or_changing_bytes` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R054` `failed_service_and_reload_actions_leave_recoverable_journal` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R055` `locked_veto_before_mutations_does_not_call_services_or_replace_source_inodes` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R056` `manual_policy_rejects_foreign_and_package_binary_without_touching_sentinels` | `tests/audit_i01_migration.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R057` `complete_baseline_and_generation_transition_examples_validate` | `tests/audit_i02_examples.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R058` `host_tunnel_uses_an_independent_source_from_the_application_group` | `tests/audit_i02_examples.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R059` `examples_contain_credential_metadata_but_no_synthetic_blob_payloads` | `tests/audit_i02_examples.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R060` `two_sources_three_apps_and_group_roundtrip_without_secret_material` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R061` `unchanged_definition_with_new_ids_preserves_sessions_without_net_block` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R062` `disappeared_definition_requires_net_evidence_for_each_live_session` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R063` `already_blocked_pin_accepts_fresh_evidence_when_definition_disappears` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R064` `existing_session_remains_pinned_after_current_tunnel_and_profile_change` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R065` `nodes_environment_browser_and_active_pins_cannot_be_rewritten` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R066` `tombstone_id_cannot_be_reused_for_same_or_different_entity_kind` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R067` `ending_one_group_session_preserves_other_session_and_host_policy` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R068` `unknown_schema_fields_and_path_ids_are_rejected_on_deserialize` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R069` `source_removal_requires_detached_references_and_durable_blob_plan` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R070` `shared_credential_survives_origin_removal_then_belongs_to_last_users_plan` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R071` `duplicate_record_fields_dictionary_ids_and_registry_ids_are_ambiguous` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R072` `net_verification_never_promotes_region_state_or_app_axes` | `tests/audit_i02_profiles.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R073` `stored_sessions_survive_read_in_a_new_process` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R074` `private_layout_and_credential_material_remain_redacted` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R075` `same_store_parallel_writers_have_one_cas_winner` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R076` `partial_blob_unlink_survives_store_reopen_and_finishes_exact_plan` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R077` `nofollow_state_rejects_symlink_and_foreign_temporary_files_are_preserved` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R078` `stale_revision_does_not_publish_credentials_or_overwrite_state` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R079` `source_update_archives_old_node_and_changes_only_net_evidence_for_live_pins` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R080` `separate_store_handles_observe_one_cas_winner_and_busy_lock_is_bounded` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R081` `source_in_use_and_immutable_node_errors_include_safe_graph_context` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R082` `corrupt_pending_blob_aborts_before_unlinking_any_planned_blob` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R083` `occupied_new_blob_is_preserved_and_old_snapshot_stays_published` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R084` `host_stop_changes_only_host_owned_tunnel_policy` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R085` `unsupported_unknown_and_duplicate_state_documents_fail_without_echoing_input` | `tests/audit_i02_store.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R086` `persisted_winner_refreshes_and_historical_node_keeps_its_artifact` | `tests/audit_i03_artifact.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R087` `changed_body_invalidates_only_net_for_lost_active_pins_and_isolates_other_sources` | `tests/audit_i03_artifact.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R088` `stale_revision_generation_and_corrupt_blob_fail_closed` | `tests/audit_i03_artifact.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R089` `local_artifact_records_local_origin_without_user_agent` | `tests/audit_i03_artifact.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R090` `typed_import_rejects_restricted_fields_defaults_and_resource_overflow` | `tests/audit_i03_artifact.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R091` `typed_protocol_and_transport_are_part_of_immutable_definition_identity` | `tests/audit_i03_artifact_review.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R092` `aggregate_payload_overflow_preserves_the_published_source_and_private_files` | `tests/audit_i03_artifact_review.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R093` `core_pin_and_import_formats_are_explicit` | `tests/audit_i03_capabilities.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R094` `protocol_transport_matrix_accepts_only_the_pinned_subset` | `tests/audit_i03_capabilities.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R095` `wrong_core_versions_and_uri_schemes_fail_with_safe_unit_errors` | `tests/audit_i03_capabilities.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R096` `native_config_policy_restricts_host_controls_and_rejects_unknown_or_advanced_fields` | `tests/audit_i03_capabilities.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R097` `accepted_corpus_is_accepted_by_pinned_core` | `tests/audit_i03_core_check.rs` | L1+L2 | SKIPPED | парсер принимает корпус; сверка с mihomo ждёт CM_TEST_MIHOMO |
| `R098` `html_stub_does_not_replace_working_source` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R099` `exhausted_or_expired_subscription_userinfo_does_not_replace_working_source` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R100` `healthy_subscription_userinfo_still_refreshes` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R101` `timeout_does_not_replace_working_source` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R102` `disappeared_nodes_do_not_replace_working_source` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R103` `failed_refresh_of_active_source_leaves_store_unchanged` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R104` `corrupt_import_does_not_replace_working_source` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R105` `odd_subscription_userinfo_does_not_block_refresh` | `tests/audit_i03_failures.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R106` `endpoint_only_change_is_private_and_keeps_generation_and_old_node_artifact` | `tests/audit_i03_fetch_settings.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R107` `settings_are_captured_before_cached_reordering_and_reject_invalid_private_records` | `tests/audit_i03_fetch_settings.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R108` `grok_t_catalogue` | `tests/audit_i03_formats.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R109` `detection_by_shape` | `tests/audit_i03_formats.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R110` `mixed_list_reports_skipped_lines_without_text` | `tests/audit_i03_formats.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R111` `list_without_importable_lines_is_refused` | `tests/audit_i03_formats.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R112` `base64_list_variants_decode_to_the_same_nodes` | `tests/audit_i03_formats.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R113` `uri_nodes_are_validated_like_native_nodes` | `tests/audit_i03_formats.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R114` `secret_in_argv_is_refused_before_anything_else` | `tests/audit_i03_manual.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R115` `stdin_and_file_secrets_are_accepted` | `tests/audit_i03_manual.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R116` `share_uri_from_stdin_builds_the_same_validated_node` | `tests/audit_i03_manual.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R117` `edits_keep_the_source_and_advance_the_generation` | `tests/audit_i03_manual.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R118` `parses_bounded_plain_tcp_nodes_and_constrained_defaults` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R119` `rejects_duplicate_json_and_nested_yaml_keys` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R120` `quoted_yaml_secret_markers_remain_plain_data_and_debug_stays_redacted` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R121` `rejects_yaml_aliases_tags_merges_and_multiple_documents_before_loading` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R122` `rejects_host_control_fields_sections_unknown_fields_and_unsupported_branches` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R123` `header_names_are_not_misclassified_as_node_controls` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R124` `validates_limits_defaults_hosts_ports_and_core_pin` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R125` `rejects_node_name_collisions_and_the_node_limit` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R126` `rejects_excessive_nesting_and_keeps_errors_secret_free` | `tests/audit_i03_native.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R127` `unusable_body_retries_and_records_actual_successful_user_agent` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R128` `cached_winner_is_first_only_when_all_cache_bindings_match` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R129` `terminal_http_and_transport_failures_do_not_retry` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R130` `candidate_request_and_time_budgets_are_bounded` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R131` `acceptance_timestamp_includes_fetch_and_classification_elapsed_time` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R132` `unix_clock_sampling_time_uses_budget_without_being_added_twice_to_timestamp` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R133` `monotonic_clock_regression_is_a_safe_terminal_error` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R134` `request_that_starts_at_deadline_is_never_fetched` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R135` `oversize_and_invalid_policy_fail_before_classifier_or_fetch` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R136` `private_values_stay_out_of_debug_output` | `tests/audit_i03_negotiation.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R137` `omissions_digest_is_stable_and_safe` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R138` `pending_summary_requires_confirmation` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R139` `wrong_confirmation_digest_is_rejected` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R140` `auto_refresh_requires_source_provenance` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R141` `omissions_bound_enforces_section_set_equality` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R142` `disabled_tls_node_cannot_hide_behind_zero_count` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R143` `auto_refresh_within_confirmed_bound_publishes` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R144` `auto_refresh_outside_bound_is_blocked` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R145` `nodes_without_tls_are_not_reported_as_verified` | `tests/audit_i03_omissions.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R146` `parsed_payload_for_different_response_cannot_be_published` | `tests/audit_i03_pipeline.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R147` `independent_parsed_sources_and_failed_refresh_preserve_other_source` | `tests/audit_i03_pipeline.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R148` `native_negotiation_retries_unusable_body_but_stops_on_unsupported_semantics` | `tests/audit_i03_pipeline.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R149` `reality_requires_servername_and_rejects_websocket` | `tests/audit_i03_protocols.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R150` `vless_flow_is_tcp_tls_only` | `tests/audit_i03_protocols.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R151` `hysteria2_and_tuic_and_trojan` | `tests/audit_i03_protocols.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R152` `shadowsocks_2022_checks_key_length_and_rejects_plugins` | `tests/audit_i03_protocols.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R153` `provenance_has_strict_schema_and_consistent_origin_time_and_body_digest` | `tests/audit_i03_provenance.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R154` `refreshing_metadata_preserves_generation_and_rejects_provenance_loss_or_reinterpretation` | `tests/audit_i03_provenance.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R155` `graph_retains_historical_core_pins_while_refusing_malformed_pin_metadata` | `tests/audit_i03_provenance.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R156` `secret_markers_stay_out_of_errors_and_debug` | `tests/audit_i03_secrets.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R157` `two_sources_are_independent_and_a_used_source_is_not_removed` | `tests/audit_i03_sources.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R158` `strict_json_rejects_duplicate_literal_keys` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R159` `strict_json_rejects_excessive_nesting` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R160` `rejects_unknown_nested_key_without_echo` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R161` `rejects_case_insensitive_duplicate_keys` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R162` `rejects_control_and_bidi_text` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R163` `accepts_and_rejects_string_length_bounds` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R164` `accepts_and_rejects_list_bounds` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R165` `rejects_invalid_scalar_types` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R166` `enforces_exclusive_and_paired_fields` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R167` `canonical_digest_is_order_independent` | `tests/audit_i03_t04_b.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R168` `wireguard_fixtures_match_grok_cases` | `tests/audit_i03_t04_m.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R169` `skip_cert_verify_without_pin_is_disabled` | `tests/audit_i03_tls.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R170` `skip_cert_verify_with_fingerprint_is_pinned` | `tests/audit_i03_tls.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R171` `false_skip_stays_verified` | `tests/audit_i03_tls.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R172` `rejects_bad_fingerprint_unknown_utls_and_host_certs` | `tests/audit_i03_tls.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R173` `accepts_ws_http_h2_grpc_and_xhttp` | `tests/audit_i03_transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R174` `rejects_bad_paths_types_and_header_injection` | `tests/audit_i03_transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R175` `user_agent_bodies_match_the_mock_retry` | `tests/audit_i03_transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R176` `redirect_chain_is_terminal_like_the_mock` | `tests/audit_i03_transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R177` `slow_response_returns_before_the_server_finishes` | `tests/audit_i03_transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R178` `oversized_body_is_not_retried` | `tests/audit_i03_transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R179` `share_uris_match_native_nodes` | `tests/audit_i03_uri.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R180` `grok_rejection_catalogue` | `tests/audit_i03_uri.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R181` `vless_security_is_never_silently_dropped` | `tests/audit_i03_uri.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R182` `transports_and_d2_insecure_map_to_native_fields` | `tests/audit_i03_uri.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R183` `http_and_socks_without_credentials_are_accepted` | `tests/audit_i03_uri.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R184` `vmess_json_transport_fields_are_mapped` | `tests/audit_i03_uri.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R185` `fake_lifecycle_keeps_readiness_axes_independent` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R186` `failed_start_leaves_the_adapter_stopped` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R187` `mihomo_descriptor_is_the_i03_tables` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R188` `two_instances_do_not_share_files_and_removal_is_local` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R189` `concurrent_leases_are_unique_and_crash_reclaim_frees_the_absent_holder` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R190` `core_unit_grants_capabilities_only_when_requested` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R191` `worker_generator_rejects_host_routing_and_foreign_listeners` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R192` `worker_refuses_every_host_listener_key` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R193` `core_unit_runs_binary_outside_writable_paths` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R194` `lease_lock_gives_up_instead_of_hanging` | `tests/audit_i04_adapter.rs` | L1 | PASS locally | процесс ядра на harness — NOT_RUN |
| `R195` `state_fixtures_cover_every_axis` | `tests/audit_i17_states.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R196` `pacman_mirrorlist_managed_only_for_arch_core` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R197` `fs01_private_temp_dir_mode_and_cleanup` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R198` `fs01_does_not_use_predictable_tmp_path` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R199` `sec04a_apt_backup_is_private` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R200` `upd05_zypper_failure_is_not_empty_list` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R201` `upd05_zypper_empty_stdout_is_ok` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R202` `upd05_zypper_parses_update_lines` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R203` `sec04b_apt_original_uri_file_is_private` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R204` `upd07_pacman_missing_exit_code_is_error` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R205` `sec04c_atomic_write_preserves_private_mode` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R206` `apt01_merge_keeps_admin_line_after_mirror_apply` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R207` `apt02_merge_rejects_orphan_mirror_list_reference` | `src/backend.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R208` `sleeping_probe_times_out_and_preserves_caller_group` | `src/common/probe.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R209` `both_streams_have_typed_limits` | `src/common/probe.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R210` `exited_leader_cannot_leave_background_work` | `src/common/probe.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R211` `cancellation_and_scope_failure_are_visible` | `src/common/probe.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R212` `setup_failure_drops_owned_child_and_group` | `src/common/probe.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R213` `net01_ipv6_gateway_decodes` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R214` `net01_ipv6_only_route_is_online` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R215` `net01_gateway_change_changes_fingerprint_id` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R216` `net01_no_default_routes_offline` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R217` `net01_parses_ipv6_default_from_fixture` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R218` `sys01_systemd_cgroup_v2_path` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R219` `sys01_systemd_cgroup_v1_path` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R220` `sys01_systemd_service_from_cgroup` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R221` `sys02_module_dirs_has_no_duplicates` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R222` `sys02_lib_modules_layout_keeps_running_kernel` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R223` `data04_unreadable_config_is_error_without_clobber` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R224` `b07_eof_is_refusal_even_with_default_yes` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R225` `b11_command_output_is_limited` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R226` `b08_config_upper_bounds` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R227` `atomic_write_16_concurrent_paths_keep_complete_content` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R228` `atomic_write_concurrent_same_path_never_exposes_mixed_content` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R229` `atomic_write_uses_exact_mode_and_replaces_symlink_entry` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R230` `atomic_write_temp_symlink_collision_does_not_follow_target` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R231` `atomic_write_failure_before_rename_preserves_target_and_cleans_temp` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R232` `atomic_write_rename_failure_preserves_target_and_cleans_temp` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R233` `atomic_write_directory_sync_error_reports_commit_stage` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R234` `every_numeric_setting_accepts_schema_bounds_and_rejects_outside` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R235` `parallel_spawn_during_pty_open_does_not_inherit_pty` | `src/common.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R236` `upd04a_missing_optional_flatpak_is_skipped` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R237` `upd04a_flatpak_error_is_not_empty_list` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R238` `upd04a_flatpak_empty_stdout_is_ok` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R239` `upd04a_flatpak_valid_line_parsed` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R240` `upd04a_flatpak_malformed_is_error` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R241` `upd04b_fwupd_command_failure_is_error` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R242` `upd04b_fwupd_empty_devices_is_ok` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R243` `upd04b_fwupd_invalid_json_is_error` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R244` `upd04b_fwupd_refresh_nothing_to_do_is_ok` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R245` `aur_search_parses_and_rejects_bad_names` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R246` `upd04c_aur_exit1_without_output_is_empty` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R247` `upd04c_aur_exit1_with_stderr_is_error` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R248` `upd04c_aur_empty_stdout_is_checked` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R249` `upd04c_aur_failure_is_not_empty_list` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R250` `upd06_flatpak_checks_user_and_system_scopes` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R251` `upd06_flatpak_checks_system_scope_without_user` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R252` `current_user_runs_directly_and_keeps_arguments` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R253` `root_switch_has_separator_and_builds_cannot_target_root` | `src/extras.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R254` `term_strips_ansi_and_rewrites_on_cr` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R255` `term_keeps_split_utf8` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R256` `stages_and_prompts` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R257` `only_listed_commands_are_allowed` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R258` `only_check_and_vpn_toggle_skip_the_password` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R259` `protocol_roundtrip_keeps_url_in_body` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R260` `operation_state_machine` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R261` `operation_ids_are_unique_within_and_across_helper_instances` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R262` `stale_prompt_from_previous_operation_is_rejected_once` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R263` `failed_prompt_write_cannot_be_replayed` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R264` `callbacks_from_old_operation_cannot_change_current_journal` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R265` `finish_emits_one_terminal_event` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R266` `inline_operation_reports_cancellation_as_unsupported` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R267` `helper_frame_accepts_one_byte_fragments` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R268` `helper_frame_limit_accepts_exact_and_rejects_one_over` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R269` `helper_frame_reports_eof_inside_json` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R270` `malformed_event_frame_is_an_error` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R271` `subscriber_coalesces_partial_and_reports_count_and_byte_overflow` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R272` `journal_and_escaped_replay_have_byte_budgets` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R273` `connection_guards_release_quotas_on_unwind` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R274` `cancellation_wakes_blocked_event_reader` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R275` `slowloris_has_absolute_request_deadline` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R276` `slow_reader_has_absolute_write_deadline` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R277` `large_vpn_list_fits_snapshot_frame_budget` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R278` `connection_spawn_failure_releases_guard` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R279` `authorization_owner_is_rechecked_after_barrier` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R280` `delayed_fake_authorizer_denies_foreign_uid_and_cannot_cancel_next_operation` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R281` `answers_are_limited_without_consuming_prompt` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R282` `full_pty_input_expires_without_blocking_state_or_cancel` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R283` `debug_request_redacts_answer` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R284` `slow_drip_reply_cannot_extend_absolute_deadline` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R285` `every_utf8_boundary_invalid_prefix_and_eof_tail_survive` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R286` `tabs_ansi_osc_and_cr_are_bounded` | `src/helper.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R287` `fill_positional_indexed_and_specs` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R288` `layouts_map_to_latin` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R289` `yes_no_many_languages` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R290` `lang_codes` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R291` `every_source_key_is_translated` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R292` `translations_keep_placeholders` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R293` `switching_language_translates` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R294` `table_has_no_duplicate_keys` | `src/i18n.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R295` `install_reloads_helper_and_reports_restart_failure` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R296` `review_binary_install_concurrency_never_mixes_files` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R297` `review_failed_notification_is_not_marked_seen_and_can_retry` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R298` `install01_package_install_requires_package_script_env` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R299` `install01_manual_layout_uses_local_bin` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R300` `install02_rejects_foreign_unit_file` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R301` `upd03_auto_failure_exit_code` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R302` `upd04a_flatpak_check_failure_affects_exit` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R303` `upd02a_flatpak_upgrade_error_is_recorded` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R304` `upd04b_firmware_check_failure_affects_exit` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R305` `upd02b_firmware_upgrade_error_preserves_first_failure` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R306` `vpn03_core_update_restart_failure_message` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R307` `install04_system_identifiers_do_not_depend_on_language` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R308` `gui01_polkit_policy_limits_passwordless_actions` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R309` `gui02_helper_socket_units_and_desktop_files` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R310` `install03_garuda_unalias_after_source_and_back` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R311` `install02_removes_owned_units_and_keeps_foreign` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R312` `vpn01_last_subscription_stops_service` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R313` `vpn01_stop_failure_is_an_error_after_delete` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R314` `upd04c_full_update_line_needs_successful_empty_aur` | `src/main.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R315` `active_legacy_vpn_helper_and_operation_admissions_are_refused` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R316` `inactive_legacy_services_are_allowed` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R317` `active_timers_paths_and_new_cm_services_remain_allowed` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R318` `live_legacy_core_is_detected_by_inode_but_same_name_is_ignored` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R319` `exact_deleted_legacy_executable_paths_are_detected` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R320` `deleted_hardlink_image_without_installed_identity_is_refused` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R321` `unreadable_or_missing_exe_for_a_present_pid_fails_closed` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R322` `missing_stat_for_present_pid_fails_closed` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R323` `missing_exe_is_ignored_only_for_kernel_threads_or_zombies` | `src/migration/manual.rs` | L1 | PASS locally | установка на хосте — NOT_RUN (X.02) |
| `R324` `b09_candidates_dedup_is_linear_and_bounded` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R325` `b08_measure_caps_threads_and_honours_deadline` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R326` `net03_rejects_html_captive_portal` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R327` `net03_accepts_minimal_apt_inrelease` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R328` `net03_rejects_truncated_inrelease` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R329` `net03_content_range_len_parses` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R330` `net02_retries_pinned_after_all_fail` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R331` `space01b_uses_existing_secondary_cache_dir` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R332` `upd01_gather_marks_update_list_failure` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R333` `space01c_statvfs_failure_surfaces_error` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R334` `net04_pending_fingerprint_on_apply_error` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R335` `space01a_unknown_size_message_is_distinct` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R336` `space01a_unknown_size_skips_prefetch` | `src/mirrors.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R337` `blob_sync_failure_keeps_old_snapshot_and_never_overwrites_orphan` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R338` `temp_sync_and_rename_failures_preserve_old_state_and_foreign_temp` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R339` `directory_sync_failure_after_rename_reports_indeterminate_and_preserves_new_state` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R340` `interrupted_unlink_resumes_exact_durable_plan_after_reopen` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R341` `corrupt_second_planned_blob_preserves_the_first_and_the_pending_snapshot` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R342` `completion_rename_failure_keeps_pending_plan_with_missing_blobs_recoverable` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R343` `v1_graph_migrates_to_v2_with_empty_omissions` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R344` `v1_graph_with_foreign_nested_version_is_refused` | `src/profiles/store/fault_tests.rs` | L1 | PASS locally | реальная ФС установки — NOT_RUN (X.03) |
| `R345` `accepts_only_declared_keys_within_bounds` | `src/sources/parser/native/opts.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R346` `output_is_independent_of_input_key_order` | `src/sources/parser/native/opts.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R347` `syntax_annotations_refused_but_secret_text_is_data` | `src/sources/parser/yaml_guard.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R348` `streaming_depth_limit_before_tree_loading` | `src/sources/parser/yaml_guard.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R349` `streaming_entry_limit_before_tree_loading` | `src/sources/parser/yaml_guard.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R350` `values_follow_the_core_schema_and_duplicates_are_refused` | `src/sources/parser/yaml_guard.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R351` `malformed_and_complex_keys_fail_without_parser_text` | `src/sources/parser/yaml_guard.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R352` `slow_dns_returns_before_budget_without_accept` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R353` `slow_loris_hits_timeout` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R354` `oversized_body_is_rejected` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R355` `redirect_chain_with_limit_zero_returns_status` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R356` `fetch_failure_display_hides_secret_url` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R357` `happy_path_reads_body` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R358` `https_downgrade_redirect_is_refused` | `src/sources/transport.rs` | L1 | PASS locally | реальный провайдер — NOT_RUN (X.04) |
| `R359` `tui_indicator_tracks_live_session_and_ignores_stale_files` | `src/summary.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R360` `badge_priority_follows_design_table` | `src/summary.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R361` `stale_data_never_reads_as_zero_updates` | `src/summary.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R362` `headline_states` | `src/summary.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R363` `low_space_is_a_warning_not_a_check_failure` | `src/summary.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R364` `mirror_problem_only_when_managed` | `src/summary.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R365` `cpu_uses_deltas_and_excludes_guest_double_counting` | `src/tui/host.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R366` `memory_counts_available_cache_as_available_and_handles_no_swap` | `src/tui/host.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R367` `network_tracks_interfaces_separately_and_excludes_loopback` | `src/tui/host.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R368` `audit_resize_ctrl_c_and_utf8_terminal_session` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R369` `stage_lines_are_parsed` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R370` `b06_closing_session_releases_reader_and_group` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R371` `b06_drop_after_reader_exit_does_not_raise_sigpipe` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R372` `review_full_pty_input_has_a_deadline_and_keeps_poll_responsive` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R373` `review_full_reader_queue_drop_joins_and_reaps_active_child` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R374` `review_reader_spawn_failure_rolls_back_child` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R375` `b06_reused_pid_is_not_our_group` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R376` `utf8_invalid_prefix_boundaries_and_eof_tail` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R377` `million_tabs_and_hundred_thousand_stages_remain_bounded` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R378` `alternate_screen_osc_cr_and_unicode_limits_are_preserved` | `src/tui/process.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R379` `installed_makepkg_translations_report_the_same_phases` | `src/tui/progress.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R380` `aur_download_build_package_and_install_are_distinct` | `src/tui/progress.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R381` `build_progress_and_unknown_durations_do_not_fake_completion` | `src/tui/progress.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R382` `main_menu_shows_host_resources_and_keeps_status_on_i` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R383` `failed_process_opens_log_and_f2_can_hide_it` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R384` `process_progress_log_and_repeated_choices_work_in_small_terminal` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R385` `test02_german_menu_and_russian_layout_hotkeys` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R386` `test01_menu_vpn_navigation_uses_fixed_state` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R387` `b04_job_runs_once_and_queues_one_repeat` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R388` `b21_lost_worker_is_reported_and_retryable` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R389` `vpn_refresh_keeps_selection_when_lists_are_reordered` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R390` `terminal_labels_have_stable_symbols_and_no_control_sequences` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R391` `narrow_inputs_keep_end_of_value_and_caret_visible` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R392` `narrow_settings_and_aur_keep_values_visible_in_all_languages` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R393` `tui_footer_and_vpn_borders_survive_resize_and_refresh` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R394` `b22_tui_confirm_is_bound_to_id` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R395` `u03_narrow_subscriptions_keep_name_and_error` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R396` `u04_empty_servers_explain_why` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R397` `u07_input_shows_hint_and_keeps_bad_value` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R398` `b16_aur_error_does_not_show_previous_results` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R399` `u05_stage_summary` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R400` `audit_minimal_tui_surfaces_keep_errors_and_keyboard_footer` | `src/tui.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R401` `auto_group_label_and_rules_follow_language` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R402` `sec01_safe_ureq_error_hides_url` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R403` `sec01_transport_error_hides_url` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R404` `sec01_scrub_stored_error_and_publish` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R405` `sec01_fetch_failure_never_leaks_subscription_url` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R406` `sec02_rejects_missing_or_bad_digest` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R407` `sec02_accepts_matching_digest` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R408` `parse01_pct_decode_utf8_percent` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R409` `parse01_pct_decode_malformed_percent_does_not_panic` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R410` `geo01_rejects_truncated_geo_file` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R411` `sec03_rejects_http_subscription_url` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R412` `sec05_strips_control_chars_from_profile_name` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R413` `vpn_rules_io_failure_is_not_empty_rules` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R414` `vpn_rules_existing_file_is_read` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R415` `data02_missing_subs_is_ok_corrupt_is_error` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R416` `vpn02_proc_ipv6_addr_parses_loopback_listener` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R417` `vpn02_listen_ports_keep_local_and_drop_foreign` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R418` `vpn03_restart_failure_is_recorded_and_success_applies_tag` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R419` `geo01_truncated_download_keeps_previous_file` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R420` `data01_subscriptions_lock_serializes` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R421` `vpn_config_lock_needs_only_writable_vpn_home` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R422` `vpn01_delete_last_subscription_clears_list` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R423` `data03_delete_sub_persists_json_before_profile_removal` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R424` `subscription_validation_failure_preserves_choice_and_config` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R425` `failed_reload_restores_disk_and_running_core` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R426` `rollback_failure_is_explicit_and_success_resolves_saved_error` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R427` `failed_commit_after_rename_restores_subscription_and_runtime` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R428` `persisted_diagnostic_redacts_profile_credentials_and_subscription_urls` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R429` `readiness_retries_and_has_a_deadline` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R430` `api_readiness_does_not_claim_remote_server_connectivity` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R431` `b22_delete_and_use_by_id_survive_list_change` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R432` `russian_auto_setting_preserves_manual_nodes_and_provider_filtering` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R433` `b01_hostile_profile_keys_are_dropped` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R434` `b01_provider_paths_are_confined` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R435` `rule_sets_provider_is_isolated_without_changing_format` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R436` `custom_http_provider_directories_are_relocated_to_cache` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R437` `http_cache_isolated_for_existing_paths_headers_and_names` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R438` `file_provider_links_follow_cache_and_ambiguous_links_fail` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R439` `local_provider_file_errors_and_symlinks_are_checked` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R440` `provider_types_formats_and_references_are_checked` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R441` `doctor_reports_each_profile_without_changing_selection_or_config` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R442` `prepare_validation_failure_keeps_previous_config` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R443` `diagnostic_hides_provider_headers_and_age_secrets` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R444` `subscription_negotiation_skips_placeholder_and_html_then_accepts_mihomo` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R445` `subscription_negotiation_accepts_uri_base64_and_stops_on_auth_failure` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R446` `unsupported_client_placeholder_and_html_are_not_valid_subscriptions` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R447` `core_validation_keeps_real_error_before_generic_failure_line` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R448` `http_provider_empty_path_uses_private_default_cache` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R449` `per_subscription_user_agent_is_validated_and_persisted_privately` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R450` `subscription_request_uses_selected_user_agent` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R451` `b23_alias_bomb_is_rejected_by_budget` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R452` `b13_port_conflict_is_reported_by_vpn_build` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R453` `b15_userinfo_counters_are_exact_and_safe` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R454` `b03_gunzip_is_limited` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R455` `b25_huge_interval_does_not_overflow` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R456` `b24_conflict_needs_real_port_or_tun` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R457` `b02_api_over_unix_socket` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |
| `R458` `b14_prepare_without_core_fails_fast` | `src/vpn.rs` | L1 | PASS locally | legacy/unit; runtime хоста не утверждается |

## Вне cargo test

| ID | Проверка | Уровень | Статус |
|---|---|---|---|
| `R-DRV-I01` | `tests/check_i01_regressions.py` без `--run` | L4 | NOT_RUN |
| `R-DRV-I02` | `tests/check_i02_regressions.py` без `--run` | L4 | NOT_RUN |
| `R-DRV-I03` | `tests/check_i03_regressions.py` без `--run` | L4 | NOT_RUN |
| `R-CORE` | `CM_TEST_MIHOMO` + `audit_i03_core_check` | L2 | SKIPPED |

Всего cargo-проверок в инвентаре: 458. Ни одна не осталась без ID и статуса.
