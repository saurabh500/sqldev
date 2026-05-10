//! Catalog SQL fragments. Centralized so the diff and explain pipelines can
//! call them too without duplicating the WHERE-clauses.

pub const SCHEMAS: &str = "
    SELECT s.schema_id, s.name
    FROM sys.schemas s
    WHERE s.name NOT IN ('sys','INFORMATION_SCHEMA','guest',
                         'db_owner','db_accessadmin','db_securityadmin',
                         'db_ddladmin','db_backupoperator','db_datareader',
                         'db_datawriter','db_denydatareader','db_denydatawriter')
    ORDER BY s.name";

pub const UDDTS: &str = "
    SELECT t.user_type_id, t.schema_id, t.name,
           bt.name AS base_name,
           t.max_length, t.precision, t.scale, t.is_nullable
    FROM sys.types t
    JOIN sys.types bt ON bt.user_type_id = t.system_type_id
    WHERE t.is_user_defined = 1 AND t.is_table_type = 0";

pub const TABLES: &str = "
    SELECT t.object_id, t.schema_id, t.name
    FROM sys.tables t
    WHERE t.is_ms_shipped = 0
    ORDER BY t.schema_id, t.name";

pub const COLUMNS: &str = "
    SELECT c.object_id, c.column_id, c.name,
           c.user_type_id,
           TYPE_NAME(c.user_type_id) AS type_base,
           c.max_length, c.precision, c.scale,
           c.is_nullable, c.is_identity,
           OBJECT_DEFINITION(c.default_object_id) AS default_def,
           cc.definition AS computed_def
    FROM sys.columns c
    LEFT JOIN sys.computed_columns cc
           ON cc.object_id = c.object_id AND cc.column_id = c.column_id
    WHERE c.object_id IN (SELECT object_id FROM sys.tables WHERE is_ms_shipped = 0)
    ORDER BY c.object_id, c.column_id";

pub const INDEX_KEY_COLS: &str = "
    SELECT ic.object_id AS parent_object_id, ic.index_id, ic.key_ordinal, c.name AS col_name
    FROM sys.index_columns ic
    JOIN sys.columns c
         ON c.object_id = ic.object_id AND c.column_id = ic.column_id
    WHERE ic.is_included_column = 0
    ORDER BY ic.object_id, ic.index_id, ic.key_ordinal";

pub const INDEX_INC_COLS: &str = "
    SELECT ic.object_id AS parent_object_id, ic.index_id, c.name AS col_name
    FROM sys.index_columns ic
    JOIN sys.columns c
         ON c.object_id = ic.object_id AND c.column_id = ic.column_id
    WHERE ic.is_included_column = 1
    ORDER BY ic.object_id, ic.index_id, ic.index_column_id";

pub const KEY_CONSTRAINTS: &str = "
    SELECT kc.parent_object_id,
           kc.name,
           kc.type AS key_type,
           i.index_id,
           i.type_desc AS index_type_desc
    FROM sys.key_constraints kc
    JOIN sys.indexes i ON i.object_id = kc.parent_object_id AND i.name = kc.name
    ORDER BY kc.parent_object_id, kc.name";

pub const INDEXES: &str = "
    SELECT i.object_id AS parent_object_id,
           i.index_id,
           i.name,
           i.is_unique,
           i.type_desc,
           i.has_filter,
           i.filter_definition
    FROM sys.indexes i
    JOIN sys.tables t ON t.object_id = i.object_id
    WHERE i.is_primary_key = 0
      AND i.is_unique_constraint = 0
      AND i.type_desc <> 'HEAP'
      AND i.name IS NOT NULL
      AND t.is_ms_shipped = 0
    ORDER BY i.object_id, i.index_id";

pub const CHECK_CONSTRAINTS: &str = "
    SELECT cc.parent_object_id, cc.name, cc.definition
    FROM sys.check_constraints cc
    ORDER BY cc.parent_object_id, cc.name";

pub const FOREIGN_KEYS: &str = "
    SELECT fk.object_id AS fk_object_id,
           fk.parent_object_id,
           fk.name,
           s2.name AS ref_schema,
           t2.name AS ref_table,
           fk.delete_referential_action_desc AS on_delete,
           fk.update_referential_action_desc AS on_update
    FROM sys.foreign_keys fk
    JOIN sys.tables t2 ON t2.object_id = fk.referenced_object_id
    JOIN sys.schemas s2 ON s2.schema_id = t2.schema_id
    ORDER BY fk.parent_object_id, fk.name";

pub const FK_COLUMNS: &str = "
    SELECT fkc.constraint_object_id,
           fkc.constraint_column_id,
           pc.name AS parent_col,
           rc.name AS referenced_col
    FROM sys.foreign_key_columns fkc
    JOIN sys.columns pc
         ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id
    JOIN sys.columns rc
         ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
    ORDER BY fkc.constraint_object_id, fkc.constraint_column_id";

pub const TRIGGERS: &str = "
    SELECT tr.parent_id AS parent_object_id,
           tr.name,
           tr.is_disabled,
           tr.is_instead_of_trigger,
           OBJECT_DEFINITION(tr.object_id) AS def
    FROM sys.triggers tr
    JOIN sys.tables t ON t.object_id = tr.parent_id
    WHERE tr.is_ms_shipped = 0 AND t.is_ms_shipped = 0
    ORDER BY tr.parent_id, tr.name";

pub const VIEWS: &str = "
    SELECT v.schema_id, v.name, OBJECT_DEFINITION(v.object_id) AS def
    FROM sys.views v
    WHERE v.is_ms_shipped = 0
    ORDER BY v.schema_id, v.name";

pub const PROCEDURES: &str = "
    SELECT p.schema_id, p.name, OBJECT_DEFINITION(p.object_id) AS def
    FROM sys.procedures p
    WHERE p.is_ms_shipped = 0
    ORDER BY p.schema_id, p.name";

pub const FUNCTIONS: &str = "
    SELECT o.schema_id, o.name, o.type AS routine_type, OBJECT_DEFINITION(o.object_id) AS def
    FROM sys.objects o
    WHERE o.type IN ('FN','IF','TF','FS','FT')
      AND o.is_ms_shipped = 0
    ORDER BY o.schema_id, o.name";
