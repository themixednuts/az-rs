use super::{
    BoundedPage, CanonicalBytes, ContentDigest, DurableCodec, DurableRecord, DurableSnapshot,
    DurableStateVersion, DurableSubject, InMemorySubject, JournalClause, JournalPage,
    JournalReadPort, JournalRecord, NonZeroU64, RecordId, RecordsAfter, ReferenceState,
    RetentionGap, SnapshotReadPort, SnapshotRequest, StorageFailure, StoredJournalRecord,
    StoredRetentionGap, journal_genesis, record_hash, snapshot_digest,
};

impl<T: DurableSubject> InMemorySubject<T> {
    /// Drops a verified journal prefix to exercise explicit retention-gap handling.
    pub fn retain_journal_from_for_conformance(&self, earliest: RecordId) {
        let mut state = self.lock();
        let Some(index) = state
            .journal
            .iter()
            .position(|record| record.id == earliest)
        else {
            return;
        };
        if index == 0 {
            return;
        }
        let earliest_version = state.journal[index].state_version;
        let required_snapshot =
            state
                .snapshots
                .range(..earliest_version)
                .next_back()
                .map(|(_, snapshot)| {
                    snapshot_digest(
                        self.id.erase(),
                        snapshot.version,
                        snapshot.journal_root,
                        &snapshot.state,
                    )
                });
        state.journal.drain(..index);
        state.retention_gap = Some(StoredRetentionGap {
            earliest_available: earliest,
            required_snapshot,
        });
    }

    /// Alters one retained payload without updating its hash for negative conformance tests.
    pub fn corrupt_journal_record_for_conformance(&self, index: usize) -> bool {
        let mut state = self.lock();
        let Some(record) = state.journal.get_mut(index) else {
            return false;
        };
        let mut bytes = record.bytes.clone().into_boxed().into_vec();
        let Some(first) = bytes.first_mut() else {
            return false;
        };
        *first ^= 0x80;
        let Ok(bytes) = CanonicalBytes::try_from_boxed(bytes.into_boxed_slice()) else {
            return false;
        };
        record.bytes = bytes;
        true
    }

    /// Assigns durable positions and hashes without mutating the published reference state.
    ///
    /// # Errors
    ///
    /// Fails closed when the per-subject sequence is exhausted.
    pub fn stage_journal(
        state: &ReferenceState,
        clauses: impl IntoIterator<Item = JournalClause>,
        state_version: DurableStateVersion,
        input_digest: ContentDigest,
        output_digest: ContentDigest,
    ) -> Result<
        (
            Vec<StoredJournalRecord>,
            u64,
            Option<RecordId>,
            ContentDigest,
        ),
        StorageFailure,
    > {
        let mut sequence = state.journal_sequence;
        let mut position = state.journal_position;
        let mut prior_hash = state.journal_root;
        let mut staged = Vec::new();
        for clause in clauses {
            sequence = sequence
                .checked_add(1)
                .ok_or(StorageFailure::RecoveryRequired)?;
            let sequence = NonZeroU64::new(sequence).ok_or(StorageFailure::RecoveryRequired)?;
            let hash = record_hash(
                sequence,
                prior_hash,
                state_version,
                input_digest,
                output_digest,
                clause.codec_id,
                clause.codec_version,
                &clause.bytes,
            );
            let id = RecordId::from_store_parts(sequence, hash);
            staged.push(StoredJournalRecord {
                id,
                state_version,
                prior_hash,
                input_digest,
                output_digest,
                codec_id: clause.codec_id,
                codec_version: clause.codec_version,
                bytes: clause.bytes,
            });
            position = Some(id);
            prior_hash = hash;
        }
        Ok((staged, sequence, position, prior_hash))
    }

    fn snapshot_sync(
        &self,
        request: SnapshotRequest<T>,
    ) -> Result<Option<DurableSnapshot<T>>, StorageFailure> {
        if request.subject() != self.id {
            return Ok(None);
        }
        let state = self.lock();
        let selected = request.at_or_before().map_or_else(
            || state.snapshots.last_key_value(),
            |version| state.snapshots.range(..=version).next_back(),
        );
        let Some((_, stored)) = selected else {
            return Ok(None);
        };
        if stored.codec_id != T::State::CODEC_ID
            || stored.state_digest
                != ContentDigest::hash("azoth durable subject state v1", stored.state.as_slice())
        {
            return Err(StorageFailure::RecoveryRequired);
        }
        let decoded = T::State::decode_exact(stored.codec_version, stored.state.as_slice())
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        Ok(Some(DurableSnapshot::from_store_snapshot(
            self.id,
            stored.version,
            stored.codec_id,
            stored.codec_version,
            decoded,
            stored.journal_position,
            stored.journal_root,
        )))
    }

    fn records_after_sync<E>(
        &self,
        request: RecordsAfter<E>,
    ) -> Result<JournalPage<E>, StorageFailure>
    where
        E: DurableRecord<Subject = T>,
    {
        if request.subject() != self.id {
            let records =
                BoundedPage::try_from_boxed(Vec::new().into_boxed_slice(), request.limit())
                    .map_err(|_| StorageFailure::RecoveryRequired)?;
            return Ok(JournalPage::from_store_page(
                records,
                None,
                journal_genesis(),
                None,
            ));
        }
        let state = self.lock();
        let gap_page = |gap: StoredRetentionGap| -> Result<JournalPage<E>, StorageFailure> {
            let records =
                BoundedPage::try_from_boxed(Vec::new().into_boxed_slice(), request.limit())
                    .map_err(|_| StorageFailure::RecoveryRequired)?;
            Ok(JournalPage::from_store_page(
                records,
                None,
                state.journal_root,
                Some(RetentionGap::new(
                    gap.earliest_available,
                    gap.required_snapshot,
                )),
            ))
        };

        let start = match request.after() {
            None => {
                if let Some(gap) = state.retention_gap {
                    return gap_page(gap);
                }
                0
            }
            Some(after) => {
                if let Some(index) = state.journal.iter().position(|record| record.id == after) {
                    index + 1
                } else if let Some(first) = state.journal.first()
                    && after
                        .sequence()
                        .get()
                        .checked_add(1)
                        .is_some_and(|next| next == first.id.sequence().get())
                    && after.hash() == first.prior_hash
                {
                    0
                } else if let Some(gap) = state.retention_gap {
                    return gap_page(gap);
                } else {
                    return Err(StorageFailure::RecoveryRequired);
                }
            }
        };
        let end = start
            .saturating_add(request.limit().get())
            .min(state.journal.len());
        let mut decoded = Vec::with_capacity(end.saturating_sub(start));
        for stored in &state.journal[start..end] {
            if stored.codec_id != E::CODEC_ID
                || stored.id.hash()
                    != record_hash(
                        stored.id.sequence(),
                        stored.prior_hash,
                        stored.state_version,
                        stored.input_digest,
                        stored.output_digest,
                        stored.codec_id,
                        stored.codec_version,
                        &stored.bytes,
                    )
            {
                return Err(StorageFailure::RecoveryRequired);
            }
            let record = E::decode_exact(stored.codec_version, stored.bytes.as_slice())
                .map_err(|_| StorageFailure::RecoveryRequired)?;
            decoded.push(JournalRecord::from_store_record(
                stored.id,
                self.id,
                stored.state_version,
                stored.codec_id,
                stored.codec_version,
                stored.prior_hash,
                stored.input_digest,
                stored.output_digest,
                stored.bytes.clone(),
                record,
            ));
        }
        let next_after = (end < state.journal.len())
            .then(|| decoded.last().map(JournalRecord::id))
            .flatten();
        let records = BoundedPage::try_from_boxed(decoded.into_boxed_slice(), request.limit())
            .map_err(|_| StorageFailure::RecoveryRequired)?;
        Ok(JournalPage::from_store_page(
            records,
            next_after,
            state.journal_root,
            None,
        ))
    }
}

impl<T: DurableSubject> SnapshotReadPort<T> for InMemorySubject<T> {
    async fn snapshot(
        &self,
        request: SnapshotRequest<T>,
    ) -> Result<Option<DurableSnapshot<T>>, StorageFailure> {
        self.snapshot_sync(request)
    }
}

impl<T, E> JournalReadPort<E> for InMemorySubject<T>
where
    T: DurableSubject,
    E: DurableRecord<Subject = T>,
{
    async fn records_after(
        &self,
        request: RecordsAfter<E>,
    ) -> Result<JournalPage<E>, StorageFailure> {
        self.records_after_sync(request)
    }
}
