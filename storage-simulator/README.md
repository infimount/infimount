# Storage Simulator

This folder provides a **complete local multi-storage testbed** for your app:

- **S3 API (SeaweedFS)**
- **POSIX Filer API (SeaweedFS)**
- **WebDAV (SeaweedFS)**
- **NFS v3 (SeaweedFS)**
- **Azure Blob (Azurite)**
- **Google Cloud Storage (Fake GCS Server)**

Everything runs via one `docker-compose.yml` and comes with a bootstrap script
that automatically creates:

- S3 bucket + sample object  
- Azure container + sample blob  
- GCS bucket + sample object  

Use this to test your **OpenDAL operators**, storage browser, metadata views,
pagination, previews, presigned URLs, versioning, and more.

---

## 🚀 Usage

### Start the simulator
```bash
make up
