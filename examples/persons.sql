-- Creates dbo.Persons in the `test` database and fills it with 1000 random people.
-- Run the whole script in querydeck (F5). Re-running drops and recreates the table.

USE test;
GO

DROP TABLE IF EXISTS dbo.Persons;
GO

CREATE TABLE dbo.Persons (
    PersonId    int IDENTITY(1,1) PRIMARY KEY,
    FirstName   nvarchar(50)  NOT NULL,
    LastName    nvarchar(50)  NOT NULL,
    Gender      char(1)       NOT NULL CHECK (Gender IN ('M', 'F', 'X')),
    BirthDate   date          NOT NULL,
    Email       nvarchar(120) NOT NULL,
    Phone       varchar(20)   NULL,
    Street      nvarchar(100) NOT NULL,
    PostalCode  varchar(10)   NOT NULL,
    City        nvarchar(60)  NOT NULL,
    Country     nvarchar(60)  NOT NULL,
    CreatedAt   datetime2(0)  NOT NULL DEFAULT SYSUTCDATETIME()
);
GO

-- Draw the random numbers into a temp table first: if NEWID() sits directly in
-- the join conditions, SQL Server may re-evaluate it and drop or duplicate rows.
DECLARE @rows int = 1000;

DROP TABLE IF EXISTS #Rand;
SELECT TOP (@rows)
    ROW_NUMBER() OVER (ORDER BY (SELECT NULL)) AS n,
    ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r1, ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r2, ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r3,
    ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r4, ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r5, ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r6,
    ABS(CAST(CHECKSUM(NEWID()) AS bigint)) AS r7
INTO #Rand
FROM sys.all_objects a CROSS JOIN sys.all_objects b;

WITH
FirstNames AS (
    SELECT * FROM (VALUES
        (1,  N'Mads',     'M'), (2,  N'Oliver',  'M'), (3,  N'Lucas',   'M'), (4,  N'William', 'M'),
        (5,  N'Noah',     'M'), (6,  N'Emil',    'M'), (7,  N'Frederik','M'), (8,  N'Magnus',  'M'),
        (9,  N'James',    'M'), (10, N'Ahmed',   'M'), (11, N'Sofia',   'F'), (12, N'Emma',    'F'),
        (13, N'Ida',      'F'), (14, N'Freja',   'F'), (15, N'Clara',   'F'), (16, N'Anna',    'F'),
        (17, N'Laura',    'F'), (18, N'Olivia',  'F'), (19, N'Maja',    'F'), (20, N'Fatima',  'F'),
        (21, N'Alex',     'X'), (22, N'Robin',   'X')
    ) v(Id, Name, Gender)
),
LastNames AS (
    SELECT * FROM (VALUES
        (1, N'Jensen'), (2, N'Nielsen'), (3, N'Hansen'), (4, N'Pedersen'), (5, N'Andersen'),
        (6, N'Christensen'), (7, N'Larsen'), (8, N'Sørensen'), (9, N'Rasmussen'), (10, N'Jørgensen'),
        (11, N'Smith'), (12, N'Johnson'), (13, N'Müller'), (14, N'García'), (15, N'Rossi'),
        (16, N'Kowalski'), (17, N'Novak'), (18, N'Dubois'), (19, N'Ali'), (20, N'Berg')
    ) v(Id, Name)
),
Streets AS (
    SELECT * FROM (VALUES
        (1, N'Vestergade'), (2, N'Nørregade'), (3, N'Strandvejen'), (4, N'Søndergade'), (5, N'Algade'),
        (6, N'Main Street'), (7, N'Hauptstraße'), (8, N'Kongensgade'), (9, N'Parkvej'), (10, N'Skovvej')
    ) v(Id, Name)
),
Cities AS (
    SELECT * FROM (VALUES
        (1,  N'København',  '1000',  N'Denmark'),
        (2,  N'Aarhus',     '8000',  N'Denmark'),
        (3,  N'Odense',     '5000',  N'Denmark'),
        (4,  N'Aalborg',    '9000',  N'Denmark'),
        (5,  N'Esbjerg',    '6700',  N'Denmark'),
        (6,  N'Malmö',      '21115', N'Sweden'),
        (7,  N'Oslo',       '0150',  N'Norway'),
        (8,  N'Hamburg',    '20095', N'Germany'),
        (9,  N'Berlin',     '10115', N'Germany'),
        (10, N'Amsterdam',  '1012',  N'Netherlands'),
        (11, N'London',     'EC1A',  N'United Kingdom'),
        (12, N'Paris',      '75001', N'France')
    ) v(Id, Name, PostalCode, Country)
)
INSERT INTO dbo.Persons
    (FirstName, LastName, Gender, BirthDate, Email, Phone, Street, PostalCode, City, Country, CreatedAt)
SELECT
    fn.Name,
    ln.Name,
    fn.Gender,
    -- Ages 18–90
    DATEADD(day, -(365 * 18 + r.r4 % (365 * 72)), CAST(GETDATE() AS date)),
    LOWER(fn.Name + N'.' + ln.Name) + CAST(r.n AS nvarchar(10)) + N'@example.com',
    -- About 10% have no phone number
    CASE WHEN r.r5 % 10 = 0 THEN NULL
         ELSE '+45 ' + RIGHT('00000000' + CAST(20000000 + r.r5 % 80000000 AS varchar(8)), 8) END,
    st.Name + N' ' + CAST(1 + r.r6 % 150 AS nvarchar(5)),
    ci.PostalCode,
    ci.Name,
    ci.Country,
    -- Created some time in the last two years
    DATEADD(minute, -(r.r7 % (60 * 24 * 730)), SYSUTCDATETIME())
FROM #Rand r
JOIN FirstNames fn ON fn.Id = 1 + r.r1 % 22
JOIN LastNames  ln ON ln.Id = 1 + r.r2 % 20
JOIN Cities     ci ON ci.Id = 1 + r.r3 % 12
JOIN Streets    st ON st.Id = 1 + r.r6 % 10;
GO

SELECT TOP (100) * FROM dbo.Persons ORDER BY PersonId;

SELECT Country, Gender, COUNT(*) AS People, MIN(BirthDate) AS Oldest, MAX(BirthDate) AS Youngest
FROM dbo.Persons
GROUP BY Country, Gender
ORDER BY Country, Gender;
