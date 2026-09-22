use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use lancedb::{
    Connection,
    arrow::{
        arrow_array::{
            FixedSizeListArray, RecordBatch, StringArray, TimestampMillisecondArray,
            types::Float64Type,
        },
        arrow_schema::{ArrowError, DataType, Field, Schema, TimeUnit},
    },
    connect,
    expr::{col, lit},
    query::{ColumnOrdering, ExecutableQuery, QueryBase},
};
use std::sync::Arc;
use uuid::Uuid;

/// Number of values required in each stored embedding vector.
const EMBEDDING_DIMENSION: i32 = 4096;

/// An indexed document chunk and its metadata, stored as one row in LanceDB.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DocumentRow {
    /// Unique identifier for this chunk's row.
    pub uuid: String,
    /// Row creation time in UTC, persisted with millisecond precision.
    pub created: DateTime<Utc>,
    /// Source file path used for exact-match lookups.
    pub filename: String,
    /// Text of the chunk represented by the embedding.
    pub content: String,
    /// Hash of the complete source file's content, used to detect changes.
    pub hash: String,
    /// Embedding vector for the chunk; must contain exactly 4096 values.
    pub embedding: Vec<f64>,
}

impl DocumentRow {
    pub fn new(filename: &str, content: &str, embedding: Vec<f64>, hash: &str) -> Self {
        Self {
            uuid: Uuid::now_v7().to_string(),
            created: chrono::Utc::now(),
            filename: filename.to_string(),
            content: content.to_string(),
            hash: hash.to_string(),
            embedding,
        }
    }

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("uuid", DataType::Utf8, false),
            Field::new(
                "created",
                DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                false,
            ),
            Field::new("filename", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("hash", DataType::Utf8, false),
            Field::new(
                "embedding",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float64, true)),
                    EMBEDDING_DIMENSION,
                ),
                false,
            ),
        ]))
    }
}

/// LanceDB storage for indexed document chunks in the `documents` table.
///
/// Call [`Storage::connect`] before inserting or querying rows. The table is
/// created on first use if absent; an existing table must match the row schema.
pub struct Storage {
    /// Active database connection
    conn: Connection,
}

impl Storage {
    /// Opens a LanceDB connection using the database path or URI in `uri`.
    ///
    /// On success, replaces any existing connection. On failure, leaves the
    /// current connection unchanged. This does not create or validate the table.
    ///
    /// # Errors
    ///
    /// Returns an error if LanceDB cannot connect to the supplied location.
    pub async fn build(uri: &str) -> Result<Self, lancedb::Error> {
        Ok(Self {
            conn: connect(uri).execute().await?,
        })
    }

    /// Appends `d` to the `documents` table.
    ///
    /// Stores the supplied creation time with millisecond precision. Existing
    /// rows with the same UUID, filename, or hash are not replaced or deduplicated.
    ///
    /// # Errors
    ///
    /// Returns an error if storage is disconnected, the table cannot be opened
    /// or created, its schema is incompatible, batch conversion fails, or the
    /// write fails.
    ///
    /// # Panics
    ///
    /// Panics if `d.embedding` does not contain exactly 4096 values.
    pub async fn insert(&self, d: &DocumentRow) -> Result<(), lancedb::Error> {
        let table = self.get_table().await?;
        let batch = self.document_to_batch(d)?;
        let _ = table.add(batch).execute().await?;
        Ok(())
    }

    /// Returns up to five document chunks nearest to `query_embedding`, ordered
    /// by increasing vector distance.
    ///
    /// The query vector must contain 4096 values and use the same embedding
    /// model as the stored vectors. Returns an empty vector if no rows match.
    ///
    /// # Errors
    ///
    /// Returns an error if the table cannot be opened or created, its schema is
    /// incompatible, query execution fails (including a vector dimension
    /// mismatch), or a result cannot be deserialized into a [`DocumentRow`].
    ///
    /// # Panics
    ///
    /// Panics if LanceDB rejects the query vector during `nearest_to`, because
    /// the query builder's result is unwrapped.
    pub async fn query(
        &self,
        query_embedding: Vec<f64>,
    ) -> Result<Vec<DocumentRow>, lancedb::Error> {
        let table = self.get_table().await?;
        let mut stream = table
            .query()
            .nearest_to(query_embedding.as_slice())
            .unwrap()
            .limit(5)
            .execute()
            .await?;

        let mut documents: Vec<DocumentRow> = Vec::new();

        while let Some(batch) = stream.try_next().await? {
            let docs: Vec<DocumentRow> =
                serde_arrow::from_record_batch(&batch).map_err(|error| {
                    lancedb::Error::External {
                        source: Box::new(error),
                    }
                })?;
            documents.extend(docs);
        }

        Ok(documents)
    }

    /// Returns at most one row whose source file path exactly matches `filename`.
    ///
    /// Selects the row with the greatest `created` timestamp, or returns an empty
    /// vector if no row matches. Ties between timestamps are not resolved in a
    /// defined order. For chunked files, this returns a single chunk rather than
    /// every chunk belonging to the file.
    ///
    /// # Errors
    ///
    /// Returns an error if storage is disconnected, the table cannot be opened
    /// or created, its schema is incompatible, the query fails, or a result
    /// cannot be deserialized into a [`DocumentRow`].
    pub async fn get_by_filename(
        &self,
        filename: &str,
    ) -> Result<Vec<DocumentRow>, lancedb::Error> {
        let table = self.get_table().await?;

        let mut stream = table
            .query()
            .only_if_expr(col("filename").eq(lit(filename.to_owned())))
            .order_by(Some(vec![ColumnOrdering::desc_nulls_last(
                "created".to_owned(),
            )]))
            .execute()
            .await?;

        let mut documents: Vec<DocumentRow> = Vec::new();

        while let Some(batch) = stream.try_next().await? {
            let docs: Vec<DocumentRow> =
                serde_arrow::from_record_batch(&batch).map_err(|error| {
                    lancedb::Error::External {
                        source: Box::new(error),
                    }
                })?;
            if let Some(document) = docs.into_iter().next() {
                documents.push(document);
            }
        }

        Ok(documents)
    }

    pub async fn delete_by_filename(&self, filename: &str) -> Result<u64, lancedb::Error> {
        let table = self.get_table().await?;
        let exp = col("filename").eq(lit(filename));
        let res = table.delete(&exp).await?;
        Ok(res.num_deleted_rows)
    }

    async fn get_table(&self) -> Result<lancedb::Table, lancedb::Error> {
        self.conn
            .create_empty_table("documents", DocumentRow::schema())
            .mode(lancedb::database::CreateTableMode::exist_ok(|request| {
                request
            }))
            .execute()
            .await
    }

    fn document_to_batch(&self, doc: &DocumentRow) -> Result<RecordBatch, ArrowError> {
        let embeddings = FixedSizeListArray::from_iter_primitive::<Float64Type, _, _>(
            [Some(doc.embedding.iter().copied().map(Some))],
            4096,
        );

        let created = TimestampMillisecondArray::from(vec![doc.created.timestamp_millis()])
            .with_timezone("UTC");

        RecordBatch::try_new(
            DocumentRow::schema(),
            vec![
                Arc::new(StringArray::from(vec![doc.uuid.as_str()])),
                Arc::new(created),
                Arc::new(StringArray::from(vec![doc.filename.as_str()])),
                Arc::new(StringArray::from(vec![doc.content.as_str()])),
                Arc::new(StringArray::from(vec![doc.hash.as_str()])),
                Arc::new(embeddings),
            ],
        )
    }
}
