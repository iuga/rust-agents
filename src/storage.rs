use futures::TryStreamExt;
use lancedb::{
    arrow::{
        arrow_array::{types::Float64Type, FixedSizeListArray, RecordBatch, StringArray},
        arrow_schema::{ArrowError, DataType, Field, Schema},
    },
    connect,
    expr::{col, lit},
    query::{ExecutableQuery, QueryBase},
    Connection,
};
use std::sync::Arc;

const EMBEDDING_DIMENSION: i32 = 4096;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DocumentRow {
    pub uuid: String,
    pub filename: String,
    pub content: String,
    pub hash: String,
    pub embedding: Vec<f64>,
}

impl DocumentRow {
    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("uuid", DataType::Utf8, false),
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

pub struct Storage {
    db: Option<Connection>,
}

impl Storage {
    pub fn new() -> Self {
        Self { db: None }
    }

    pub async fn connect(&mut self, uri: &str) -> Result<(), lancedb::Error> {
        let connection = connect(uri).execute().await?;
        self.db = Some(connection);
        Ok(())
    }

    pub async fn insert(&self, d: &DocumentRow) -> Result<(), lancedb::Error> {
        let table = self.get_table().await?;
        let batch = self.document_to_batch(&d)?;
        let result = table.add(batch).execute().await?;
        println!("> {:?}", result);
        Ok(())
    }

    pub async fn get_by_filename(&self, filename: &str) -> Result<Vec<DocumentRow>, lancedb::Error> {
        let table = self.get_table().await?;

        let mut stream = table
            .query()
            .only_if_expr(col("filename").eq(lit(filename.to_owned())))
            .limit(1)
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

    async fn get_table(&self) -> Result<lancedb::Table, lancedb::Error> {
        let db = self.db.as_ref().ok_or_else(|| lancedb::Error::Runtime {
            message: "Storage is not connected".into(),
        })?;

        db.create_empty_table("documents", DocumentRow::schema())
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
        RecordBatch::try_new(
            DocumentRow::schema(),
            vec![
                Arc::new(StringArray::from(vec![doc.uuid.as_str()])),
                Arc::new(StringArray::from(vec![doc.filename.as_str()])),
                Arc::new(StringArray::from(vec![doc.content.as_str()])),
                Arc::new(StringArray::from(vec![doc.hash.as_str()])),
                Arc::new(embeddings),
            ],
        )
    }
}
