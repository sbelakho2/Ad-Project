//! Blob storage for message bodies.

use anyhow::Result;
use mail_common::config::StorageConfig;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::info;

/// Blob storage for immutable message bodies.
#[derive(Clone)]
pub struct BlobStorage {
    base_path: PathBuf,
    compress: bool,
}

impl BlobStorage {
    /// Create a new blob storage instance.
    pub fn new(config: &StorageConfig) -> Result<Self> {
        let base_path = config.blob_path.clone();
        
        // Ensure directory exists
        std::fs::create_dir_all(&base_path)?;
        
        info!("Blob storage initialized at {:?}", base_path);
        
        Ok(Self {
            base_path,
            compress: config.compress,
        })
    }
    
    /// Store a message blob and return its hash.
    pub async fn store(&self, data: &[u8]) -> Result<String> {
        // Compute hash
        let hash = Self::compute_hash(data);
        let path = self.hash_to_path(&hash);
        
        // Check if already exists (deduplication)
        if path.exists() {
            return Ok(hash);
        }
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        
        // Write data
        let data_to_write = if self.compress {
            Self::compress(data)?
        } else {
            data.to_vec()
        };
        
        tokio::fs::write(&path, data_to_write).await?;
        
        Ok(hash)
    }
    
    /// Retrieve a message blob by hash.
    pub async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>> {
        let path = self.hash_to_path(hash);
        
        if !path.exists() {
            return Ok(None);
        }
        
        let data = tokio::fs::read(&path).await?;
        
        let result = if self.compress {
            Self::decompress(&data)?
        } else {
            data
        };
        
        Ok(Some(result))
    }
    
    /// Delete a blob by hash.
    pub async fn delete(&self, hash: &str) -> Result<bool> {
        let path = self.hash_to_path(hash);
        
        if !path.exists() {
            return Ok(false);
        }
        
        tokio::fs::remove_file(&path).await?;
        Ok(true)
    }
    
    /// Check if a blob exists.
    pub async fn exists(&self, hash: &str) -> bool {
        self.hash_to_path(hash).exists()
    }
    
    /// Get the size of a blob.
    pub async fn size(&self, hash: &str) -> Result<Option<u64>> {
        let path = self.hash_to_path(hash);
        
        match tokio::fs::metadata(&path).await {
            Ok(meta) => Ok(Some(meta.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    /// Compute SHA-256 hash of data.
    pub fn compute_hash(data: &[u8]) -> String {
        let hash = Sha256::digest(data);
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            hash,
        )
    }
    
    /// Convert hash to filesystem path (sharded by first 2 chars).
    fn hash_to_path(&self, hash: &str) -> PathBuf {
        // Shard by first 2 characters for better filesystem performance
        let shard = &hash[..2.min(hash.len())];
        self.base_path.join(shard).join(hash)
    }
    
    /// Compress data using zstd.
    fn compress(data: &[u8]) -> Result<Vec<u8>> {
        let mut encoder = zstd::Encoder::new(Vec::new(), 3)?;
        encoder.write_all(data)?;
        Ok(encoder.finish()?)
    }
    
    /// Decompress zstd data.
    fn decompress(data: &[u8]) -> Result<Vec<u8>> {
        let mut decoder = zstd::Decoder::new(data)?;
        let mut result = Vec::new();
        std::io::copy(&mut decoder, &mut result)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_store_and_retrieve() {
        let temp = TempDir::new().unwrap();
        let config = StorageConfig {
            blob_path: temp.path().to_path_buf(),
            index_path: temp.path().to_path_buf(),
            deduplicate: true,
            compress: false,
        };
        
        let storage = BlobStorage::new(&config).unwrap();
        
        let data = b"Hello, World!";
        let hash = storage.store(data).await.unwrap();
        
        let retrieved = storage.get(&hash).await.unwrap().unwrap();
        assert_eq!(retrieved, data);
    }
    
    #[tokio::test]
    async fn test_deduplication() {
        let temp = TempDir::new().unwrap();
        let config = StorageConfig {
            blob_path: temp.path().to_path_buf(),
            index_path: temp.path().to_path_buf(),
            deduplicate: true,
            compress: false,
        };
        
        let storage = BlobStorage::new(&config).unwrap();
        
        let data = b"Duplicate data";
        let hash1 = storage.store(data).await.unwrap();
        let hash2 = storage.store(data).await.unwrap();
        
        assert_eq!(hash1, hash2);
    }
}
